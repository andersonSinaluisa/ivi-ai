//! Suite de seguridad del threat model (HU-WIN-049).
//!
//! Una prueba por amenaza mitigable de [docs/04-security/threat-model.md], con el nombre de la
//! amenaza en el test para que la trazabilidad se vea sin abrir la documentación. El atacante es
//! siempre el mismo: un **Cloud comprometido** que puede publicar cualquier mensaje bien formado
//! en el topic de este device.

mod common;

use std::time::Duration;

use common::{Harness, LaunchBehaviour};
use jarvis_policy::Policy;
use jarvis_protocol::{Channel, Envelope};
use serde_json::{Value, json};
use uuid::Uuid;

use common::OWNER;
const T: Duration = Duration::from_secs(10);

fn owner() -> String {
    format!("cloud:user:{OWNER}")
}

/// Espera breve para comprobar que algo **no** ocurre.
async fn quiet() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

// ---- WT-01 Compromised Cloud -------------------------------------------------------

#[tokio::test]
async fn wt01_a_compromised_cloud_cannot_run_r3_without_local_approval() {
    let harness = Harness::new();
    harness.windows.add_running(1234, "notepad.exe", true);
    harness.windows.set_ignores_close(true);

    let task = Uuid::now_v7();
    harness.dispatch(task, "CLOSE_APP", json!({"appId": "notepad", "force": true}), &owner()).await;

    harness.wait_for("task.approval_required", T).await;
    quiet().await;
    assert_eq!(harness.windows.terminated(), Vec::<u32>::new(), "R3 no se ejecuta sin aprobación local");
    assert_eq!(harness.publisher.count("task.completed"), 0);
}

#[tokio::test]
async fn wt01_an_unknown_principal_is_never_obeyed() {
    let harness = Harness::new();
    let task = Uuid::now_v7();
    // Un usuario que no está en la política local.
    harness
        .dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), "cloud:user:0192a6f0-9999-7aaa-8bbb-999999999999")
        .await;

    let rejected = harness.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], json!("NOT_PERMITTED"), "{:?}", rejected.data);
    assert_eq!(harness.publisher.count("task.started"), 0);
}

#[tokio::test]
async fn wt01_an_app_outside_the_local_catalog_is_refused() {
    let harness = Harness::new();
    let task = Uuid::now_v7();
    harness.dispatch(task, "OPEN_APP", json!({"appId": "malware-installer"}), &owner()).await;

    let rejected = harness.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], json!("INVALID_PAYLOAD"), "{:?}", rejected.data);
}

// ---- WT-02 / WT-04 mensajes ajenos y repetidos -------------------------------------

#[test]
fn wt02_topics_of_other_devices_are_not_even_parsed() {
    // Primera barrera del Cloud Link: solo se aceptan los topics `down/*` de este device. La
    // segunda (el `deviceId` del envelope) está en `CloudLink::on_message`, justo antes de
    // entregar el mensaje al pipeline.
    use jarvis_protocol::Topics;
    let mine = Uuid::now_v7();
    let other = Uuid::now_v7();
    let topics = Topics::new(mine);

    assert!(topics.parse_own_down(&format!("jarvis/v1/devices/{mine}/down/tasks")).is_some());
    for hostile in [
        format!("jarvis/v1/devices/{other}/down/tasks"),
        format!("jarvis/v1/devices/{mine}/up/tasks"),
        format!("jarvis/v1/devices/{mine}/down/../../{other}/down/tasks"),
        format!("jarvis/v2/devices/{mine}/down/tasks"),
        "jarvis/v1/devices/+/down/tasks".to_owned(),
        "#".to_owned(),
    ] {
        assert!(topics.parse_own_down(&hostile).is_none(), "debe rechazarse: {hostile}");
    }
}

#[tokio::test]
async fn wt04_replaying_an_executed_dispatch_never_executes_it_again() {
    let harness = Harness::new();
    let task = Uuid::now_v7();
    let envelope = harness.dispatch_envelope(task, "OPEN_APP", json!({"appId": "notepad"}), &owner(), 300, None);

    harness.agent.handle_envelope(Channel::Tasks, envelope.clone()).await;
    harness.wait_for("task.completed", T).await;

    for _ in 0..5 {
        harness.agent.handle_envelope(Channel::Tasks, envelope.clone()).await;
    }
    quiet().await;
    assert_eq!(harness.publisher.count("task.started"), 1, "una sola ejecución");
    let execution = harness.agent.store.execution(task).expect("query").expect("row");
    assert_eq!(execution.status, "SUCCEEDED");
}

#[tokio::test]
async fn wt04_a_task_created_before_the_local_database_is_refused() {
    let harness = Harness::new();
    let task = Uuid::now_v7();
    let mut envelope = harness.dispatch_envelope(task, "OPEN_APP", json!({"appId": "notepad"}), &owner(), 300, None);
    if let Some(data) = envelope.data.as_object_mut() {
        // Tarea de "hace un año": anterior a esta base local.
        data.insert("createdAt".to_owned(), json!("2025-09-19T10:00:00.000Z"));
    }

    harness.agent.handle_envelope(Channel::Tasks, envelope).await;
    let rejected = harness.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], json!("EXPIRED_ON_ARRIVAL"), "{:?}", rejected.data);
}

// ---- WT-05 Task tampering ----------------------------------------------------------

#[tokio::test]
async fn wt05_raising_the_payload_risk_raises_the_decision_too() {
    let harness = Harness::new();
    harness.windows.add_running(1234, "notepad.exe", true);

    // `force: true` sube close_app de R2 a R3: el agente lo evalúa por el input real, no por lo
    // que diga el Cloud sobre él.
    let task = Uuid::now_v7();
    harness.dispatch(task, "CLOSE_APP", json!({"appId": "notepad", "force": true}), &owner()).await;

    let required = harness.wait_for("task.approval_required", T).await;
    assert!(required.data["reason"].as_str().unwrap_or_default().contains("R3"), "{:?}", required.data);
    // La aprobacion queda atada al payload exacto que se vio: con otro input, otro hash.
    // El hash es el canonico del payload (hex de 64 caracteres), el mismo que firmaria un
    // aprobador remoto; si cambia el formato, las decisiones firmadas dejan de cuadrar.
    let approval = harness.agent.store.approval(task).expect("query").expect("row");
    assert_eq!(
        approval.payload_hash,
        jarvis_protocol::signature::payload_hash(&json!({"appId": "notepad", "force": true})),
        "el hash de la aprobacion debe ser el canonico del payload"
    );

    let other = Uuid::now_v7();
    harness.dispatch(other, "CLOSE_APP", json!({"appId": "notepad", "force": false}), &owner()).await;
    quiet().await;
    if let Ok(Some(second)) = harness.agent.store.approval(other) {
        assert_ne!(second.payload_hash, approval.payload_hash, "payloads distintos, aprobaciones distintas");
    }
}

// ---- WT-07 Command injection -------------------------------------------------------

#[tokio::test]
async fn wt07_injection_attempts_in_app_id_are_rejected() {
    let harness = Harness::new();
    let hostile = [
        json!({"appId": "chrome & calc.exe"}),
        json!({"appId": "chrome | calc"}),
        json!({"appId": "chrome; calc"}),
        json!({"appId": "$(whoami)"}),
        json!({"appId": "`whoami`"}),
        json!({"appId": "../../windows/system32/cmd.exe"}),
        json!({"appId": r"C:\Windows\System32\cmd.exe"}),
        json!({"appId": "chrome\u{0}calc"}),
        json!({"appId": "сhrome"}), // 'с' cirílica: no es la entrada del catálogo
        json!({"appId": "notepad", "args": ["--evil"]}),
        json!({"appId": "notepad", "command": "calc"}),
    ];
    for payload in hostile {
        harness.publisher.clear();
        harness.dispatch(Uuid::now_v7(), "OPEN_APP", payload.clone(), &owner()).await;
        let rejected = harness.wait_for("task.rejected", T).await;
        assert_eq!(rejected.data["error"]["code"], json!("INVALID_PAYLOAD"), "{payload} → {:?}", rejected.data);
    }
}

// ---- WT-09 App impersonation -------------------------------------------------------

#[tokio::test]
async fn wt09_a_process_with_a_known_name_in_an_untrusted_folder_is_not_the_app() {
    // El doble de plataforma decide qué ubicación es de confianza; aquí se comprueba que la tool
    // pregunta por ello en vez de fiarse del nombre del ejecutable.
    let harness = Harness::new();
    // Un `notepad.exe` que vive en %TEMP%, no donde lo registra el catalogo.
    harness.windows.add_running_at(1234, "notepad.exe", r"C:\Users\victima\AppData\Local\Temp\notepad.exe", true);
    harness.windows.set_trusted(false);

    let task = Uuid::now_v7();
    harness.dispatch(task, "IS_APP_RUNNING", json!({"appId": "notepad"}), &owner()).await;
    let completed = harness.wait_for("task.completed", T).await;
    assert_eq!(completed.data["result"]["output"]["running"], json!(false), "un impostor no cuenta como la app");
}

// ---- WT-15 Approval spoofing -------------------------------------------------------

#[tokio::test]
async fn wt15_an_approval_arriving_by_mqtt_without_signature_is_ignored() {
    let harness = Harness::new();
    harness.windows.add_running(1234, "notepad.exe", true);
    harness.windows.set_ignores_close(true);

    let task = Uuid::now_v7();
    harness.dispatch(task, "CLOSE_APP", json!({"appId": "notepad", "force": true}), &owner()).await;
    harness.wait_for("task.approval_required", T).await;

    // Un "Cloud malicioso" intenta aprobarse a sí mismo de varias formas.
    let forged = [
        ("task.approval_decision", json!({"taskId": task, "decision": "GRANTED"})),
        ("task.approved", json!({"taskId": task})),
        ("approval.granted", json!({"taskId": task, "granted": true})),
    ];
    for (kind, data) in forged {
        let envelope = Envelope::create(kind, harness.device_id, &data, None, None).expect("envelope");
        harness.agent.handle_envelope(Channel::Tasks, envelope).await;
    }
    quiet().await;

    let approval = harness.agent.store.approval(task).expect("query").expect("row");
    assert_eq!(approval.state, jarvis_state::ApprovalState::Pending, "la aprobación sigue esperando al owner");
    assert_eq!(harness.windows.terminated(), Vec::<u32>::new());
}

// ---- WT-14 / WT-12 datos sensibles --------------------------------------------------

#[tokio::test]
async fn wt14_sensitive_output_is_redacted_from_the_local_database_after_settling() {
    let harness = Harness::new();
    harness.windows.add_running(1234, "code.exe", true);

    let task = Uuid::now_v7();
    harness.dispatch(task, "GET_ACTIVE_WINDOW", json!({"includeTitle": true}), &owner()).await;
    let completed = harness.wait_for("task.completed", T).await;
    // El título sí viaja al Cloud (es lo que se pidió) …
    assert!(format!("{}", completed.data).contains("Visual Studio Code"));

    // … pero no se conserva en la base tras asentarse.
    harness.ack(task).await;
    let stored = format!("{:?}", harness.agent.store.execution(task).expect("query"));
    assert!(!stored.contains("Visual Studio Code"), "el título se redacta tras el settle");
}

#[tokio::test]
async fn wt12_error_messages_never_carry_the_user_profile_path() {
    let harness = Harness::new();
    harness.windows.set_launch(LaunchBehaviour::NotFound);

    let task = Uuid::now_v7();
    harness.dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), &owner()).await;
    let failed = harness.wait_for("task.failed", T).await;

    let text = format!("{}", failed.data);
    if let Ok(profile) = std::env::var("USERPROFILE") {
        assert!(!text.contains(&profile), "el mensaje no lleva la ruta del perfil: {text}");
    }
    assert!(!text.contains("panicked"), "ni trazas internas: {text}");
}

// ---- política local por encima del Cloud -------------------------------------------

#[tokio::test]
async fn an_invalid_local_policy_fails_closed() {
    // Política que no se puede cargar → solo R0; el Cloud no gana permisos por ello.
    let harness = Harness::with_policy(Policy::default());
    let task = Uuid::now_v7();
    harness.dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), &owner()).await;

    let rejected = harness.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], json!("NOT_PERMITTED"), "{:?}", rejected.data);
}

#[tokio::test]
async fn r4_is_never_allowed_remotely_even_with_capabilities() {
    // No hay tools R4 en el catálogo actual: se comprueba en la capa de decisión, que es donde
    // vive la regla, para que el día que exista una tool R4 el límite ya esté puesto.
    use jarvis_policy::{Authenticity, DecisionKind, PolicyState, Principal};
    let policy = Policy::parse(&format!(
        r#"
version = 1
[[principals]]
id = "cloud:user:{OWNER}"
capabilities = ["windows.apps.close", "windows.apps.close.force"]
"#
    ))
    .expect("policy");
    let state = PolicyState { policy, valid: true };
    let decision = jarvis_policy::evaluate(
        &state,
        &Principal::CloudUser(Uuid::parse_str(OWNER).expect("uuid")),
        "delete_file",
        &Value::Null,
        jarvis_tooling::RiskLevel::R4,
        &["windows.apps.close".to_owned()],
        &Authenticity::Claimed,
    );
    assert_eq!(decision.kind, DecisionKind::Denied);
    assert_eq!(decision.reason_code, "CRITICAL_REMOTE");
}
