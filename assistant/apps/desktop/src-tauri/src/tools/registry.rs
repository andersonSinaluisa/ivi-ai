//! Registro de herramientas: única fuente de lo que el LLM puede pedir.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;

use super::Tool;
use crate::domain::{ToolSource, ToolSpec};
use crate::infrastructure::llm::LlmTool;

#[derive(Default)]
pub struct ToolRegistry {
    tools: RwLock<BTreeMap<String, Arc<dyn Tool>>>,
}

/// Comprueba que un descriptor sea utilizable.
///
/// # Errors
/// Motivo del rechazo.
pub fn validate_spec(spec: &ToolSpec) -> Result<(), String> {
    let name_ok = !spec.name.is_empty()
        && spec.name.len() <= 64
        && spec.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-');
    if !name_ok {
        return Err(format!("nombre de herramienta inválido: '{}'", spec.name));
    }
    if spec.id.is_empty() {
        return Err(format!("{}: falta el identificador", spec.name));
    }
    if spec.timeout_ms == 0 || spec.timeout_ms > 10 * 60 * 1000 {
        return Err(format!("{}: el tiempo máximo debe estar entre 1 ms y 10 min", spec.name));
    }
    if spec.parameters.get("type").and_then(Value::as_str) != Some("object") {
        return Err(format!("{}: el esquema de parámetros debe ser de tipo object", spec.name));
    }
    if spec.description.trim().is_empty() {
        return Err(format!("{}: falta la descripción", spec.name));
    }
    Ok(())
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    /// Si el descriptor no es válido o el nombre ya existe.
    pub fn register(&self, tool: Arc<dyn Tool>) -> Result<(), String> {
        let spec = tool.spec();
        validate_spec(spec)?;
        let mut tools = self.tools.write().map_err(|_| "registro envenenado".to_owned())?;
        if tools.contains_key(&spec.name) {
            return Err(format!("herramienta duplicada: '{}'", spec.name));
        }
        tools.insert(spec.name.clone(), tool);
        Ok(())
    }

    /// Sustituye todas las herramientas de un origen (al reconectar con el windows-agent).
    /// Las locales tienen prioridad: una remota con el mismo nombre se ignora.
    pub fn replace_source(&self, source: ToolSource, tools: Vec<Arc<dyn Tool>>) -> Vec<String> {
        let mut rejected = Vec::new();
        let Ok(mut map) = self.tools.write() else { return rejected };
        map.retain(|_, t| t.spec().source != source);
        for tool in tools {
            let spec = tool.spec();
            if let Err(e) = validate_spec(spec) {
                rejected.push(e);
            } else if map.contains_key(&spec.name) {
                rejected.push(format!("'{}' ya existe; se ignora la versión de {:?}", spec.name, source));
            } else {
                map.insert(spec.name.clone(), tool);
            }
        }
        rejected
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().ok()?.get(name).cloned()
    }

    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.read().map(|t| t.values().map(|t| t.spec().clone()).collect()).unwrap_or_default()
    }

    #[must_use]
    pub fn count(&self, source: ToolSource) -> usize {
        self.tools.read().map(|t| t.values().filter(|t| t.spec().source == source).count()).unwrap_or(0)
    }

    /// Descripción para el modelo. Se omiten las herramientas que la política deniega.
    #[must_use]
    pub fn llm_tools(&self, is_denied: &dyn Fn(&ToolSpec) -> bool) -> Vec<LlmTool> {
        self.specs()
            .into_iter()
            .filter(|s| !is_denied(s))
            .map(|s| LlmTool {
                name: s.name.clone(),
                description: format!("{} [riesgo: {}]", s.description, s.risk.as_str()),
                parameters: s.parameters.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::local::{GetCurrentTime, GetSystemInfo};

    #[test]
    fn registers_and_rejects_duplicates() {
        let r = ToolRegistry::new();
        r.register(Arc::new(GetCurrentTime::new())).expect("time");
        r.register(Arc::new(GetSystemInfo::new())).expect("sys");
        assert!(r.register(Arc::new(GetCurrentTime::new())).unwrap_err().contains("duplicada"));
        assert_eq!(r.count(ToolSource::Local), 2);
        assert!(r.get("get_current_time").is_some());
        assert!(r.get("rm_rf").is_none());
    }

    #[test]
    fn llm_tools_exclude_denied_ones() {
        let r = ToolRegistry::new();
        r.register(Arc::new(GetCurrentTime::new())).expect("time");
        r.register(Arc::new(GetSystemInfo::new())).expect("sys");
        let tools = r.llm_tools(&|s| s.name == "get_system_info");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "get_current_time");
        assert!(tools[0].description.contains("READ_ONLY"));
    }

    #[test]
    fn invalid_specs_are_rejected() {
        let mut spec = GetCurrentTime::new().spec().clone();
        spec.name = "con espacios".into();
        assert!(validate_spec(&spec).is_err());
        let mut spec = GetCurrentTime::new().spec().clone();
        spec.timeout_ms = 0;
        assert!(validate_spec(&spec).is_err());
        let mut spec = GetCurrentTime::new().spec().clone();
        spec.parameters = serde_json::json!({"type": "string"});
        assert!(validate_spec(&spec).is_err());
    }
}
