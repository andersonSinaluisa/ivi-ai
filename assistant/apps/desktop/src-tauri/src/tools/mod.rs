//! Sistema de herramientas. El LLM solo *propone* llamadas; el `ToolRegistry` resuelve el nombre
//! a una implementación y el `PolicyEngine` decide si se ejecuta.

pub mod local;
pub mod registry;
pub mod remote;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::domain::{ToolOutput, ToolSpec};

pub use registry::ToolRegistry;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ToolError {
    #[error("parámetros inválidos: {0}")]
    InvalidArguments(String),
    #[error("la herramienta superó su tiempo máximo ({0} ms)")]
    Timeout(u64),
    #[error("cancelada")]
    Cancelled,
    #[error("no permitido: {0}")]
    NotPermitted(String),
    #[error("no disponible: {0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub run_id: String,
    pub call_id: String,
    pub cancel: CancellationToken,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;

    /// Valida y normaliza los argumentos antes de autorizar.
    ///
    /// # Errors
    /// [`ToolError::InvalidArguments`] si no cumplen el esquema.
    fn validate(&self, arguments: &Value) -> Result<Value, ToolError>;

    async fn execute(&self, arguments: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}

/// Validación mínima de JSON Schema para objetos: tipo, `required`, `additionalProperties: false`
/// y tipos primitivos de cada propiedad. Las herramientas remotas se validan además en el
/// windows-agent con su esquema completo.
///
/// # Errors
/// Mensaje con la primera discrepancia encontrada.
pub fn validate_object(schema: &Value, arguments: &Value) -> Result<(), String> {
    let args = match arguments {
        Value::Object(map) => map,
        Value::Null => return validate_object(schema, &Value::Object(serde_json::Map::new())),
        _ => return Err("los argumentos deben ser un objeto JSON".into()),
    };
    let props = schema.get("properties").and_then(Value::as_object);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for r in required.iter().filter_map(Value::as_str) {
            if !args.contains_key(r) {
                return Err(format!("falta el parámetro obligatorio '{r}'"));
            }
        }
    }
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    for (k, v) in args {
        let Some(prop) = props.and_then(|p| p.get(k)) else {
            if closed {
                return Err(format!("parámetro desconocido '{k}'"));
            }
            continue;
        };
        if let Some(t) = prop.get("type").and_then(Value::as_str) {
            let ok = match t {
                "string" => v.is_string(),
                "integer" => v.is_i64() || v.is_u64(),
                "number" => v.is_number(),
                "boolean" => v.is_boolean(),
                "array" => v.is_array(),
                "object" => v.is_object(),
                _ => true,
            };
            if !ok {
                return Err(format!("'{k}' debe ser de tipo {t}"));
            }
        }
        if let Some(options) = prop.get("enum").and_then(Value::as_array) {
            if !options.contains(v) {
                return Err(format!("'{k}' no es un valor permitido"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_object;
    use serde_json::json;

    #[test]
    fn validates_required_types_and_closed_objects() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}, "limit": {"type": "integer"}, "mode": {"enum": ["a", "b"]}},
            "required": ["path"],
            "additionalProperties": false
        });
        assert!(validate_object(&schema, &json!({"path": "C:/"})).is_ok());
        assert!(validate_object(&schema, &json!({})).unwrap_err().contains("path"));
        assert!(validate_object(&schema, &json!({"path": 3})).unwrap_err().contains("string"));
        assert!(validate_object(&schema, &json!({"path": "x", "rm": true})).unwrap_err().contains("desconocido"));
        assert!(validate_object(&schema, &json!({"path": "x", "mode": "c"})).is_err());
        assert!(validate_object(&schema, &json!("texto")).is_err());
    }
}
