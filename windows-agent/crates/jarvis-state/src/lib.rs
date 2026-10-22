//! Estado local durable (ADR-005, docs/06-state).
//!
//! Garantías: una sola ejecución por `taskId` (UNIQUE estructural), estado y mensaje de outbox en la
//! misma transacción, y durabilidad ante corte de energía (`synchronous=FULL`).
//!
//! El crate no conoce el protocolo: guarda los envelopes ya serializados por el agente.

pub mod model;
mod schema;
pub mod store;

pub use model::{
    Approval, ApprovalState, ExecutionRecord, LocalState, NewTask, OutboxMessage, PendingApproval, StoreStats, TaskRow,
};
pub use store::{Store, StoreError};

/// Hash del payload (para detectar dispatches con el mismo `taskId` y contenido distinto).
#[must_use]
pub fn payload_hash(payload: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_vec(payload).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(&canonical))
}
