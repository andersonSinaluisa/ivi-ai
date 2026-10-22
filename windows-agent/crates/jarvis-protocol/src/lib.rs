//! Protocolo MQTT v1 de Jarvis Cloud, lado device.
//!
//! Fuente de verdad: `api/docs/03-protocols/mqtt-protocol.md` y los contratos C# de
//! `api/src/Jarvis.Contracts`. Este crate no redefine el contrato: lo refleja en tipos Rust.
//! Diferencias documentadas en `docs/00-overview/cloud-compatibility.md`.

pub mod codes;
pub mod envelope;
pub mod messages;
pub mod signature;
pub mod time;
pub mod topics;

pub use envelope::{Envelope, EnvelopeError, MAX_MESSAGE_BYTES};
pub use signature::{PublicKey, Signature, SignatureError};
pub use topics::{Channel, Direction, Topics};
