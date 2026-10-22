//! Tipos del estado local.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Estados locales de una tarea (docs/01-architecture/execution-pipeline.md#estados-locales).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalState {
    Rejected,
    AwaitingApproval,
    Accepted,
    Running,
    Verifying,
    Succeeded,
    Failed,
    Cancelled,
}

impl LocalState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rejected => "Rejected",
            Self::AwaitingApproval => "AwaitingApproval",
            Self::Accepted => "Accepted",
            Self::Running => "Running",
            Self::Verifying => "Verifying",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "Rejected" => Self::Rejected,
            "AwaitingApproval" => Self::AwaitingApproval,
            "Accepted" => Self::Accepted,
            "Running" => Self::Running,
            "Verifying" => Self::Verifying,
            "Succeeded" => Self::Succeeded,
            "Failed" => Self::Failed,
            "Cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// Mensaje a publicar (ya serializado). Se persiste junto al cambio de estado que lo origina.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxMessage {
    pub message_type: String,
    pub envelope_json: String,
}

impl OutboxMessage {
    #[must_use]
    pub fn new(message_type: impl Into<String>, envelope_json: impl Into<String>) -> Self {
        Self { message_type: message_type.into(), envelope_json: envelope_json.into() }
    }
}

/// Tarea aceptada o rechazada que se persiste antes de publicar nada.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub task_id: Uuid,
    pub correlation_id: Option<Uuid>,
    pub task_type: String,
    pub tool: Option<String>,
    pub principal: String,
    pub payload: serde_json::Value,
    pub state: LocalState,
    pub risk_level: Option<String>,
    pub decision: Option<String>,
    pub decision_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_duration_seconds: Option<u32>,
    /// La tool declara datos sensibles: el resultado se redacta tras *settled*.
    pub sensitive: bool,
    /// `keyId` de la firma válida que traía la tarea; `None` si llegó sin firmar (P8).
    pub signed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub task_id: Uuid,
    pub correlation_id: Option<Uuid>,
    pub task_type: String,
    pub tool: Option<String>,
    pub principal: String,
    pub payload: serde_json::Value,
    pub payload_hash: String,
    pub state: LocalState,
    pub settled: bool,
    pub risk_level: Option<String>,
    pub decision: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_duration_seconds: Option<u32>,
    pub sensitive: bool,
    /// `keyId` de la firma con la que llegó la tarea, si venía firmada.
    pub signed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionRecord {
    pub execution_id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub verification_status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result_json: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    Pending,
    Granted,
    Denied,
    Expired,
}

impl ApprovalState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Granted => "Granted",
            Self::Denied => "Denied",
            Self::Expired => "Expired",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "Pending" => Self::Pending,
            "Granted" => Self::Granted,
            "Denied" => Self::Denied,
            "Expired" => Self::Expired,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub task_id: Uuid,
    pub state: ApprovalState,
    pub payload_hash: String,
    pub expires_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decided_by: Option<String>,
}

/// Aprobación pendiente con el contexto que necesita el owner para decidir.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingApproval {
    pub approval: Approval,
    pub tool: Option<String>,
    pub task_type: String,
    pub principal: String,
    pub risk_level: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoreStats {
    pub tasks: u64,
    pub outbox_pending: u64,
    pub unsettled: u64,
    pub oldest_unsettled_age_seconds: Option<i64>,
    pub size_bytes: u64,
}
