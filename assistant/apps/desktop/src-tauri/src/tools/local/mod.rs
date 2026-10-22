//! Herramientas locales: información del sistema y memoria. Funcionan aunque el windows-agent no
//! esté corriendo.
//! Las acciones sobre Windows (abrir apps, archivos, procesos, ventanas) viven en el
//! windows-agent y llegan como herramientas remotas.

mod memory_tools;
mod system_info;
mod time;

pub use memory_tools::{SaveMemory, SearchMemory};
pub use system_info::GetSystemInfo;
pub use time::GetCurrentTime;

use serde_json::Value;

/// Esquema incrustado desde `packages/tool-schemas`.
pub(crate) fn schema(text: &str) -> Value {
    let mut v: Value = serde_json::from_str(text).unwrap_or_else(|_| serde_json::json!({"type": "object"}));
    // El modelo no necesita los metadatos del esquema.
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
        obj.remove("description");
    }
    v
}
