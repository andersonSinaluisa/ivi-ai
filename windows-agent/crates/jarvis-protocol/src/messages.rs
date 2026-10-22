//! `data` de cada tipo de mensaje usado por el agente (espejo de `Jarvis.Contracts.Mqtt.MessageData`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---- down/tasks ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestedBy {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatch {
    pub task_id: Uuid,
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(default = "empty_object")]
    pub payload: serde_json::Value,
    #[serde(default = "one")]
    pub attempt: u32,
    #[serde(with = "crate::time::millis")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "crate::time::millis")]
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub max_duration_seconds: Option<u32>,
    pub requested_by: RequestedBy,
    #[serde(default)]
    pub cancel_requested: bool,
    /// Firma del originador. El Cloud la manda `null` en v1 (C10); cuando llegue, se verifica.
    #[serde(default)]
    pub signature: Option<crate::signature::Signature>,
}

/// Decisión de aprobación firmada por el dispositivo del owner (docs/04-security/approvals.md).
///
/// La firma **no** la hace el Cloud: solo transporta el mensaje. Sin firma válida se ignora.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecision {
    pub task_id: Uuid,
    /// `GRANTED` o `DENIED`.
    pub decision: String,
    pub decided_by: RequestedBy,
    #[serde(with = "crate::time::millis")]
    pub decided_at: DateTime<Utc>,
    /// Único por decisión: una firma capturada no se puede reutilizar.
    pub nonce: String,
    pub signature: crate::signature::Signature,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

const fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCancel {
    pub task_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdateAck {
    pub task_id: Uuid,
    #[serde(default)]
    pub status: Option<String>,
}

// ---- down/control ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceHelloAck {
    pub session_id: String,
    #[serde(with = "crate::time::millis")]
    pub server_time: DateTime<Utc>,
    pub heartbeat_interval_seconds: u32,
    #[serde(default)]
    pub max_message_bytes: Option<u32>,
}

// ---- up/presence ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceHello {
    pub session_id: String,
    pub protocol_version: String,
    pub software_version: String,
    pub platform: String,
    pub capabilities: Vec<String>,
    pub supported_task_types: Vec<String>,
    pub open_task_ids: Vec<Uuid>,
    pub busy: bool,
    /// Extensión aditiva del protocolo 1.1 (C11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceHeartbeat {
    pub session_id: String,
    pub busy: bool,
    pub open_task_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceGoodbye {
    pub session_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceOffline {
    pub session_id: String,
}

// ---- up/tasks ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRef {
    pub task_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRejected {
    pub task_id: Uuid,
    pub error: ErrorInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStarted {
    pub task_id: Uuid,
    #[serde(with = "crate::time::millis")]
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub task_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskApprovalRequired {
    pub task_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCompleted {
    pub task_id: Uuid,
    #[serde(with = "crate::time::millis")]
    pub completed_at: DateTime<Utc>,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFailed {
    pub task_id: Uuid,
    pub error: ErrorInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCancelled {
    pub task_id: Uuid,
    pub reason: String,
}
