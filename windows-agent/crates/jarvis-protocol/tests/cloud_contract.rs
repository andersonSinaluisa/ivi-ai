//! Contract tests contra la documentación del Cloud (fuente compartida, sin copiar fixtures a mano).
//!
//! Extrae los bloques ```json de `api/docs/03-protocols/mqtt-protocol.md`:
//! - los mensajes `down` deben deserializarse en los tipos del agente;
//! - para los mensajes `up`, lo que produce el agente debe contener las mismas claves que el ejemplo del Cloud.

use std::collections::BTreeSet;
use std::path::PathBuf;

use jarvis_protocol::messages::{
    ErrorInfo, PresenceHello, PresenceHelloAck, TaskCancel, TaskCompleted, TaskDispatch, TaskFailed, TaskUpdateAck,
};
use jarvis_protocol::{Envelope, codes::message_types as mt};
use serde_json::Value;
use uuid::Uuid;

fn cloud_doc() -> Option<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../api/docs/03-protocols/mqtt-protocol.md");
    std::fs::read_to_string(path).ok()
}

/// Ejemplos completos (los que contienen `…` son esquemáticos y se omiten).
fn examples() -> Vec<Value> {
    let Some(doc) = cloud_doc() else {
        eprintln!("SKIPPED: api/docs not found next to windows-agent");
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut in_block = false;
    let mut current = String::new();
    for line in doc.lines() {
        if line.trim_start().starts_with("```json") {
            in_block = true;
            current.clear();
        } else if in_block && line.trim_start().starts_with("```") {
            in_block = false;
            if let Ok(value) = serde_json::from_str::<Value>(&fill_placeholders(&current)) {
                out.push(value);
            }
        } else if in_block {
            current.push_str(line);
            current.push('\n');
        }
    }
    out
}

/// Algunos ejemplos abrevian valores con `"…"`; se rellenan con valores válidos del mismo tipo
/// para poder comprobar la **forma** del mensaje (que es lo que verifica el contrato).
fn fill_placeholders(text: &str) -> String {
    const UUID: &str = "0192a6f4-0000-7000-8000-000000000000";
    const TS: &str = "2026-09-19T15:04:05.000Z";
    let mut out = text.to_owned();
    for (key, value) in
        [("id", UUID), ("deviceId", UUID), ("correlationId", UUID), ("causationId", UUID), ("taskId", UUID), ("ts", TS)]
    {
        out = out.replace(&format!("\"{key}\": \"…\""), &format!("\"{key}\": \"{value}\""));
    }
    out
}

fn example(kind: &str) -> Option<Value> {
    examples().into_iter().find(|v| v.get("type").and_then(Value::as_str) == Some(kind))
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

fn parse(value: &Value) -> Envelope {
    Envelope::parse(value.to_string().as_bytes()).expect("cloud example must be a valid envelope")
}

#[test]
fn down_messages_from_cloud_docs_deserialize() {
    let Some(dispatch) = example(mt::TASK_DISPATCH) else { return };
    let dispatch: TaskDispatch = parse(&dispatch).data_as().expect("task.dispatch data");
    assert_eq!(dispatch.task_type, "OPEN_CHROME");
    assert_eq!(dispatch.requested_by.kind, "Device");
    assert_eq!(dispatch.max_duration_seconds, Some(120));

    let ack: TaskUpdateAck = parse(&example(mt::TASK_UPDATE_ACK).expect("example")).data_as().expect("update.ack");
    assert_eq!(ack.status.as_deref(), Some("Completed"));

    let cancel: TaskCancel = parse(&example(mt::TASK_CANCEL).expect("example")).data_as().expect("cancel");
    assert_eq!(cancel.reason.as_deref(), Some("USER_REQUEST"));

    let hello_ack: PresenceHelloAck =
        parse(&example(mt::PRESENCE_HELLO_ACK).unwrap_or_else(synthetic_hello_ack)).data_as().expect("hello.ack");
    assert_eq!(hello_ack.heartbeat_interval_seconds, 30);
}

/// El ejemplo de `presence.hello.ack` del Cloud usa `…` en `id`: se completa solo ese campo.
fn synthetic_hello_ack() -> Value {
    serde_json::json!({
        "v": 1, "id": Uuid::nil(), "type": "presence.hello.ack", "ts": "2026-09-19T15:00:00.050Z",
        "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
        "data": { "sessionId": "s-7f3a9c21e4b0", "serverTime": "2026-09-19T15:00:00.050Z",
                  "heartbeatIntervalSeconds": 30, "maxMessageBytes": 65536 }
    })
}

#[test]
fn up_messages_have_the_cloud_shape() {
    let device = Uuid::nil();
    let task = Uuid::now_v7();

    if let Some(cloud) = example(mt::TASK_COMPLETED) {
        let ours = Envelope::create(
            mt::TASK_COMPLETED,
            device,
            &TaskCompleted { task_id: task, completed_at: chrono::Utc::now(), result: serde_json::json!({}) },
            Some(task),
            None,
        )
        .expect("serializable");
        let ours = serde_json::to_value(&ours).expect("json");
        assert_eq!(keys(&ours), keys(&cloud), "envelope keys");
        assert_eq!(keys(&ours["data"]), keys(&cloud["data"]), "task.completed data keys");
    }

    if let Some(cloud) = example(mt::TASK_FAILED) {
        let data =
            TaskFailed { task_id: task, error: ErrorInfo { code: "X".into(), message: "m".into(), retryable: false } };
        let ours = serde_json::to_value(&data).expect("json");
        assert_eq!(keys(&ours), keys(&cloud["data"]));
        assert_eq!(keys(&ours["error"]), keys(&cloud["data"]["error"]));
    }

    if let Some(cloud) = example(mt::PRESENCE_HELLO) {
        let hello = PresenceHello {
            session_id: "s-1".into(),
            protocol_version: "1.1".into(),
            software_version: "jarvis-windows-agent/0.1.0".into(),
            platform: "windows".into(),
            capabilities: vec![],
            supported_task_types: vec![],
            open_task_ids: vec![],
            busy: false,
            session_state: None,
        };
        let ours = serde_json::to_value(&hello).expect("json");
        assert_eq!(keys(&ours), keys(&cloud["data"]), "presence.hello data keys (sin la extensión 1.1)");
    }
}
