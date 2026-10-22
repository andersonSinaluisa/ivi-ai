//! Tipos de mensaje y códigos de error del catálogo del Cloud usados en el cable.

pub mod message_types {
    pub const TASK_DISPATCH: &str = "task.dispatch";
    pub const TASK_CANCEL: &str = "task.cancel";
    pub const TASK_UPDATE_ACK: &str = "task.update.ack";
    /// Decisión de aprobación firmada por el dispositivo del owner (P8; el Cloud solo la transporta).
    pub const TASK_APPROVAL_DECISION: &str = "task.approval_decision";
    pub const PRESENCE_HELLO_ACK: &str = "presence.hello.ack";

    pub const TASK_ACK: &str = "task.ack";
    pub const TASK_REJECTED: &str = "task.rejected";
    pub const TASK_STARTED: &str = "task.started";
    pub const TASK_PROGRESS: &str = "task.progress";
    pub const TASK_APPROVAL_REQUIRED: &str = "task.approval_required";
    pub const TASK_COMPLETED: &str = "task.completed";
    pub const TASK_FAILED: &str = "task.failed";
    pub const TASK_CANCELLED: &str = "task.cancelled";

    pub const PRESENCE_HELLO: &str = "presence.hello";
    pub const PRESENCE_HEARTBEAT: &str = "presence.heartbeat";
    pub const PRESENCE_GOODBYE: &str = "presence.goodbye";
    pub const PRESENCE_OFFLINE: &str = "presence.offline";
}

/// Códigos de `task.rejected` (catálogo del Cloud; el detalle fino va en `message`, ver C5).
pub mod reject_codes {
    pub const EXPIRED_ON_ARRIVAL: &str = "EXPIRED_ON_ARRIVAL";
    pub const UNSUPPORTED_TASK_TYPE: &str = "UNSUPPORTED_TASK_TYPE";
    pub const INVALID_PAYLOAD: &str = "INVALID_PAYLOAD";
    pub const NOT_PERMITTED: &str = "NOT_PERMITTED";
}

/// Razones de `task.cancelled`.
pub mod cancel_reasons {
    pub const USER_REQUEST: &str = "USER_REQUEST";
    pub const APPROVAL_DENIED: &str = "APPROVAL_DENIED";
    /// Extensión documentada en C9 (el Cloud guarda cualquier razón).
    pub const APPROVAL_EXPIRED: &str = "APPROVAL_EXPIRED";
    pub const LOCAL_POLICY: &str = "LOCAL_POLICY";
}
