//! Sincronización con Jarvis Cloud Core (`jarvis.sync/1`, ADR-013 … ADR-015 del Cloud).
//!
//! Local primero: todo cambio se guarda en SQLite y un trigger lo anota en `sync_outbox` en la misma transacción.
//! Con conexión, [`engine::SyncEngine`] envía el outbox, recibe los cambios de otros devices y deja los conflictos
//! a la vista. Sin conexión, Jarvis sigue funcionando y sincroniza después.

pub mod client;
pub mod engine;
pub mod protocol;
pub mod retrieval;
pub mod secrets;
pub mod store;

pub use engine::{SyncEngine, SyncMode, SyncReport, SyncStatus};
