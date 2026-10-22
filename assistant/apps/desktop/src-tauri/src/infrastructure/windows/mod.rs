//! Integración con el windows-agent existente (`ivi/windows-agent`) por su Local Tool API.
//!
//! Según el ADR-014 de ese proyecto, el agent core **no** llama a Win32 directamente: todas las
//! acciones sobre Windows pasan por el pipe `\\.\pipe\jarvis-tools-<SID>` (JSON-RPC 2.0), donde el
//! windows-agent aplica su propia política, verificación y auditoría con el principal
//! `local:agent-core`.

pub mod tool_api;

pub use tool_api::{RemoteToolDescriptor, ToolApiClient, ToolApiError};
