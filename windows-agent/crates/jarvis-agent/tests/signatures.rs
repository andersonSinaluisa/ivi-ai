//! Firmas de tareas y aprobaciones remotas firmadas (HU-WIN-061, HU-WIN-062).
//!
//! El "Cloud malicioso" de estos tests es el propio test: construye mensajes con firmas ausentes,
//! ajenas, alteradas o repetidas, y comprueba que ninguno consigue que el agente actúe.

mod common;

use std::time::Duration;

use chrono::Utc;
use common::Harness;
use ed25519_dalek::{Signer, SigningKey};
use jarvis_policy::Policy;
use jarvis_protocol::messages::{ApprovalDecision, RequestedBy};
use jarvis_protocol::signature::{self, Signature};
use jarvis_protocol::{Channel, Envelope};
use serde_json::{Value, json};
use uuid::Uuid;

const OWNER: &str = "0192a6f0-1111-7aaa-8bbb-000000000001";
const KEY_ID: &str = "cloud-signer-1";
const APPROVER_KEY_ID: &str = "owner-phone-1";

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - index * 6)) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn cloud_key() -> SigningKey {
    SigningKey::from_bytes(&[11u8; 32])
}

fn approver_key() -> SigningKey {
    SigningKey::from_bytes(&[22u8; 32])
}

/// Otra clave cualquiera: la de un atacante que no está registrada.
fn impostor_key() -> SigningKey {
    SigningKey::from_bytes(&[99u8; 32])
}

fn sign(key: &SigningKey, key_id: &str, message: &str) -> Signature {
    Signature {
        alg: "Ed25519".to_owned(),
        key_id: key_id.to_owned(),
        value: encode_base64(&key.sign(message.as_bytes()).to_bytes()),
    }
}

/// Política que exige firma a partir de R2 y registra las claves del Cloud y del aprobador.
fn signing_policy(require_above: &str) -> Policy {
    let text = format!(
        r#"
version = 1
[[principals]]
id = "cloud:user:{OWNER}"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close", "windows.apps.close.force"]
public_keys = [{{ id = "{KEY_ID}", key = "{}" }}]

[[principals]]
id = "local:owner"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close"]

[signatures]
require_above = "{require_above}"
approver_keys = [{{ id = "{APPROVER_KEY_ID}", key = "{}" }}]
"#,
        encode_base64(cloud_key().verifying_key().as_bytes()),
        encode_base64(approver_key().verifying_key().as_bytes()),
    );
    Policy::parse(&text).expect("valid signing policy")
}

/// Despacha una tarea sin firma.
async fn dispatch_unsigned(harness: &Harness, task_id: Uuid, payload: Value) {
    let envelope = task_envelope(harness, task_id, payload);
    harness.agent.handle_envelope(Channel::Tasks, envelope).await;
}

fn task_envelope(harness: &Harness, task_id: Uuid, payload: Value) -> Envelope {
    harness.dispatch_envelope(task_id, "OPEN_APP", payload, &format!("cloud:user:{OWNER}"), 300, None)
}

/// Despacha una tarea firmada. `signed_payload` permite firmar una cosa y enviar otra, que es lo
/// que intentaría quien reaprovecha una firma legítima.
async fn dispatch_signed_with(
    harness: &Harness,
    task_id: Uuid,
    payload: Value,
    signed_payload: &Value,
    key: &SigningKey,
    key_id: &str,
) {
    // La firma se calcula sobre **este** envelope: si se firmara otro, el `expiresAt` no cuadraría.
    let mut envelope = task_envelope(harness, task_id, payload);
    let expires_at = envelope.data["expiresAt"].as_str().expect("expiresAt");
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at).expect("rfc3339").with_timezone(&Utc);
    let message = signature::task_message(
        task_id,
        "OPEN_APP",
        signed_payload,
        &RequestedBy { kind: "User".to_owned(), id: Uuid::parse_str(OWNER).expect("uuid") },
        expires_at,
    );
    let signature = sign(key, key_id, &message);
    if let Some(data) = envelope.data.as_object_mut() {
        data.insert("signature".to_owned(), serde_json::to_value(signature).expect("signature"));
    }
    harness.agent.handle_envelope(Channel::Tasks, envelope).await;
}

/// Da tiempo a que el agente procese un mensaje que **no** debe producir nada observable.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

// ---- HU-WIN-061: firma de tareas ---------------------------------------------------

#[tokio::test]
async fn an_unsigned_task_above_the_threshold_is_denied() {
    let harness = Harness::with_policy(signing_policy("R2"));
    let task = Uuid::now_v7();
    dispatch_unsigned(&harness, task, json!({"appId": "notepad"})).await;
    let rejected = harness.wait_for("task.rejected", Duration::from_secs(5)).await;

    assert_eq!(rejected.data["error"]["code"], json!("NOT_PERMITTED"), "{:?}", rejected.data);
    assert!(
        rejected.data["error"]["message"].as_str().unwrap_or_default().contains("signed"),
        "el mensaje dice que falta firma: {:?}",
        rejected.data
    );
}

#[tokio::test]
async fn a_correctly_signed_task_runs() {
    let harness = Harness::with_policy(signing_policy("R2"));
    let task = Uuid::now_v7();
    let payload = json!({"appId": "notepad"});
    dispatch_signed_with(&harness, task, payload.clone(), &payload, &cloud_key(), KEY_ID).await;
    let completed = harness.wait_for("task.completed", Duration::from_secs(10)).await;
    assert_eq!(completed.data["result"]["outcome"], json!("SUCCEEDED"));
}

#[tokio::test]
async fn a_signature_from_an_unregistered_key_is_rejected() {
    let harness = Harness::with_policy(signing_policy("R2"));
    let task = Uuid::now_v7();
    let payload = json!({"appId": "notepad"});
    // Mismo keyId, otra clave privada: es lo que haría quien copia el formato sin tener la clave.
    dispatch_signed_with(&harness, task, payload.clone(), &payload, &impostor_key(), KEY_ID).await;
    let rejected = harness.wait_for("task.rejected", Duration::from_secs(5)).await;
    assert_eq!(rejected.data["error"]["code"], json!("NOT_PERMITTED"), "{:?}", rejected.data);
    assert_eq!(harness.publisher.count("task.started"), 0, "no se ejecutó nada");
}

#[tokio::test]
async fn a_signature_for_another_payload_does_not_transfer() {
    let harness = Harness::with_policy(signing_policy("R2"));
    let task = Uuid::now_v7();
    // Firma legítima para abrir el bloc de notas, reutilizada para abrir otra aplicación.
    dispatch_signed_with(
        &harness,
        task,
        json!({"appId": "chrome"}),
        &json!({"appId": "notepad"}),
        &cloud_key(),
        KEY_ID,
    )
    .await;

    let rejected = harness.wait_for("task.rejected", Duration::from_secs(5)).await;
    assert_eq!(rejected.data["error"]["code"], json!("NOT_PERMITTED"), "{:?}", rejected.data);
}

#[tokio::test]
async fn below_the_threshold_an_unsigned_task_still_works() {
    let harness = Harness::with_policy(signing_policy("R3"));
    let task = Uuid::now_v7();
    // `open_app` es R2: por debajo del umbral R3, sigue sin exigirse firma.
    dispatch_unsigned(&harness, task, json!({"appId": "notepad"})).await;
    let completed = harness.wait_for("task.completed", Duration::from_secs(10)).await;
    assert_eq!(completed.data["result"]["outcome"], json!("SUCCEEDED"));
}

#[tokio::test]
async fn an_invalid_signature_is_rejected_even_below_the_threshold() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let task = Uuid::now_v7();
    let payload = json!({"appId": "notepad"});
    // Aunque la política no exigiera firma, traer una que no cuadra es peor señal que no traerla.
    dispatch_signed_with(&harness, task, payload.clone(), &payload, &impostor_key(), KEY_ID).await;
    let rejected = harness.wait_for("task.rejected", Duration::from_secs(5)).await;
    assert_eq!(rejected.data["error"]["code"], json!("NOT_PERMITTED"), "{:?}", rejected.data);
}

// ---- HU-WIN-062: aprobación remota firmada -----------------------------------------

/// Deja una tarea R3 esperando aprobación y devuelve `(taskId, payloadHash)`.
///
/// El hash se calcula **desde el payload**, como hace quien firma en remoto, y no se lee de la
/// base del agente: si el agente guardara otra cosa (otro formato, o el input ya mapeado), ningún
/// aprobador externo podría firmar una decisión válida y estas pruebas deben verlo.
async fn pending_approval(harness: &Harness) -> (Uuid, String) {
    harness.windows.add_running(1234, "notepad.exe", true);
    harness.windows.set_ignores_close(true);
    let task = Uuid::now_v7();
    let payload = json!({"appId": "notepad", "force": true});
    let principal = format!("cloud:user:{OWNER}");
    let envelope = harness.dispatch_envelope(task, "CLOSE_APP", payload.clone(), &principal, 300, None);
    harness.agent.handle_envelope(Channel::Tasks, envelope).await;
    harness.wait_for("task.approval_required", Duration::from_secs(5)).await;

    harness.agent.store.approval(task).expect("query").expect("pending");
    (task, signature::payload_hash(&payload))
}

fn decision_envelope(harness: &Harness, decision: ApprovalDecision) -> Envelope {
    Envelope::create("task.approval_decision", harness.device_id, &decision, None, None).expect("envelope")
}

fn decision(
    task: Uuid,
    payload_hash: &str,
    nonce: &str,
    key: &SigningKey,
    key_id: &str,
    granted: bool,
) -> ApprovalDecision {
    let decided_at = Utc::now();
    let verdict = if granted { "GRANTED" } else { "DENIED" };
    let message = signature::decision_message(task, verdict, payload_hash, nonce, decided_at);
    ApprovalDecision {
        task_id: task,
        decision: verdict.to_owned(),
        decided_by: RequestedBy { kind: "User".to_owned(), id: Uuid::parse_str(OWNER).expect("uuid") },
        decided_at,
        nonce: nonce.to_owned(),
        signature: sign(key, key_id, &message),
    }
}

#[tokio::test]
async fn a_signed_remote_decision_approves_the_task() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let (task, payload_hash) = pending_approval(&harness).await;

    let decision = decision(task, &payload_hash, "nonce-a1", &approver_key(), APPROVER_KEY_ID, true);
    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, decision)).await;

    harness.wait_for("task.completed", Duration::from_secs(20)).await;
    let approval = harness.agent.store.approval(task).expect("query").expect("row");
    assert_eq!(approval.decided_by.as_deref(), Some("remote:owner-phone-1"), "queda quién decidió");
}

#[tokio::test]
async fn a_decision_signed_by_an_unregistered_key_changes_nothing() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let (task, payload_hash) = pending_approval(&harness).await;

    let forged = decision(task, &payload_hash, "nonce-b1", &impostor_key(), APPROVER_KEY_ID, true);
    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, forged)).await;
    settle().await;

    let approval = harness.agent.store.approval(task).expect("query").expect("row");
    assert_eq!(approval.state, jarvis_state::ApprovalState::Pending, "la aprobación sigue pendiente");
    assert_eq!(harness.publisher.count("task.started"), 0);
}

#[tokio::test]
async fn a_decision_for_another_payload_is_ignored() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let (task, _) = pending_approval(&harness).await;

    // Firma válida, pero sobre el hash de otro payload: no cubre esta tarea.
    let wrong = decision(
        task,
        "0000000000000000000000000000000000000000000000000000000000000000",
        "nonce-c1",
        &approver_key(),
        APPROVER_KEY_ID,
        true,
    );
    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, wrong)).await;
    settle().await;

    assert_eq!(
        harness.agent.store.approval(task).expect("query").expect("row").state,
        jarvis_state::ApprovalState::Pending
    );
}

#[tokio::test]
async fn a_captured_decision_cannot_be_replayed() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let (first, hash_first) = pending_approval(&harness).await;

    let granted = decision(first, &hash_first, "nonce-d1", &approver_key(), APPROVER_KEY_ID, true);
    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, granted.clone())).await;
    harness.wait_for("task.completed", Duration::from_secs(20)).await;

    // Reenviar exactamente la misma decisión no vuelve a decidir nada.
    harness.publisher.clear();
    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, granted)).await;
    settle().await;
    assert_eq!(harness.publisher.count("task.started"), 0, "el nonce ya estaba usado");
}

#[tokio::test]
async fn a_stale_decision_is_ignored() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let (task, payload_hash) = pending_approval(&harness).await;

    let mut old = decision(task, &payload_hash, "nonce-e1", &approver_key(), APPROVER_KEY_ID, true);
    // Firmada hace una hora: fuera de la ventana de frescura, aunque la firma sea correcta.
    old.decided_at = Utc::now() - chrono::Duration::hours(1);
    let message = signature::decision_message(task, &old.decision, &payload_hash, &old.nonce, old.decided_at);
    old.signature = sign(&approver_key(), APPROVER_KEY_ID, &message);

    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, old)).await;
    settle().await;
    assert_eq!(
        harness.agent.store.approval(task).expect("query").expect("row").state,
        jarvis_state::ApprovalState::Pending
    );
}

#[tokio::test]
async fn a_signed_denial_cancels_the_task() {
    let harness = Harness::with_policy(signing_policy("R4"));
    let (task, payload_hash) = pending_approval(&harness).await;

    let denied = decision(task, &payload_hash, "nonce-f1", &approver_key(), APPROVER_KEY_ID, false);
    harness.agent.handle_envelope(Channel::Tasks, decision_envelope(&harness, denied)).await;

    let cancelled = harness.wait_for("task.cancelled", Duration::from_secs(5)).await;
    assert_eq!(cancelled.data["reason"], json!("APPROVAL_DENIED"));
    assert_eq!(harness.windows.terminated(), Vec::<u32>::new(), "no se forzó el cierre");
}
