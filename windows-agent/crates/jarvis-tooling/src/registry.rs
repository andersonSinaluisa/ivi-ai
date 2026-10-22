//! Registro de tools y mapeo TaskType ↔ tool (docs/03-tools/tool-registry.md, ADR-015).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::Value;

use crate::descriptor::ToolDescriptor;
use crate::tool::Tool;

/// Alias de TaskType hacia una tool, con transformación del payload.
#[derive(Clone)]
pub struct Alias {
    pub task_type: &'static str,
    pub tool: &'static str,
    pub transform: fn(&Value) -> Value,
}

/// Tool resuelta con el input que se le pasará (ya transformado si vino por alias).
#[derive(Clone)]
pub struct Resolved {
    pub tool: Arc<dyn Tool>,
    pub input: Value,
    pub alias: Option<&'static str>,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved").field("tool", &self.tool.descriptor().name).field("alias", &self.alias).finish()
    }
}

/// Registro inmutable: se construye al arrancar y no cambia durante la vida del proceso.
pub struct ToolRegistry {
    tools: BTreeMap<&'static str, Arc<dyn Tool>>,
    aliases: BTreeMap<&'static str, Alias>,
    enabled: BTreeSet<String>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry").field("tools", &self.tools.keys()).field("enabled", &self.enabled).finish()
    }
}

#[derive(Default)]
pub struct RegistryBuilder {
    tools: Vec<Arc<dyn Tool>>,
    aliases: Vec<Alias>,
}

impl RegistryBuilder {
    #[must_use]
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    #[must_use]
    pub fn alias(mut self, alias: Alias) -> Self {
        self.aliases.push(alias);
        self
    }

    /// Valida descriptores, nombres únicos, alias y la lista de habilitadas.
    ///
    /// # Errors
    /// Mensaje con el problema; el agente no debe arrancar con un registro inválido.
    pub fn build(self, enabled: &[String]) -> Result<ToolRegistry, String> {
        let mut tools = BTreeMap::new();
        for tool in self.tools {
            let descriptor = tool.descriptor();
            descriptor.check()?;
            if tools.insert(descriptor.name, tool.clone()).is_some() {
                return Err(format!("duplicate tool name '{}'", descriptor.name));
            }
        }
        let mut aliases = BTreeMap::new();
        for alias in self.aliases {
            if !is_task_type(alias.task_type) {
                return Err(format!("alias '{}' is not a valid TaskType", alias.task_type));
            }
            if !tools.contains_key(alias.tool) {
                return Err(format!("alias '{}' targets unknown tool '{}'", alias.task_type, alias.tool));
            }
            if tools.contains_key(alias.task_type.to_ascii_lowercase().as_str()) {
                return Err(format!("alias '{}' shadows a tool", alias.task_type));
            }
            aliases.insert(alias.task_type, alias);
        }
        let mut enabled_set = BTreeSet::new();
        for name in enabled {
            if !tools.contains_key(name.as_str()) {
                return Err(format!("enabled tool '{name}' does not exist"));
            }
            enabled_set.insert(name.clone());
        }
        Ok(ToolRegistry { tools, aliases, enabled: enabled_set })
    }
}

/// `UPPER_SNAKE_CASE`, 1..64 caracteres, empieza por letra (formato `TaskType` del Cloud).
#[must_use]
pub fn is_task_type(text: &str) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_uppercase()
        && bytes.iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
}

impl ToolRegistry {
    #[must_use]
    pub fn builder() -> RegistryBuilder {
        RegistryBuilder::default()
    }

    fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(name)
    }

    /// Resuelve un TaskType del Cloud. `None` = `UNSUPPORTED_TASK_TYPE` (desconocido o deshabilitado).
    #[must_use]
    pub fn resolve_task_type(&self, task_type: &str, payload: &Value) -> Option<Resolved> {
        if !is_task_type(task_type) {
            return None;
        }
        if let Some(alias) = self.aliases.get(task_type) {
            let tool = self.tools.get(alias.tool).filter(|_| self.is_enabled(alias.tool))?;
            return Some(Resolved {
                tool: tool.clone(),
                input: (alias.transform)(payload),
                alias: Some(alias.task_type),
            });
        }
        self.resolve_tool(&task_type.to_ascii_lowercase(), payload.clone())
    }

    /// Resuelve por nombre de tool (frontend local).
    #[must_use]
    pub fn resolve_tool(&self, name: &str, input: Value) -> Option<Resolved> {
        let tool = self.tools.get(name).filter(|_| self.is_enabled(name))?;
        Some(Resolved { tool: tool.clone(), input, alias: None })
    }

    /// `supportedTaskTypes` del `presence.hello`: tools habilitadas + alias cuyo destino está habilitado.
    #[must_use]
    pub fn supported_task_types(&self) -> Vec<String> {
        let mut types: Vec<String> = self.enabled.iter().map(|n| n.to_ascii_uppercase()).collect();
        types.extend(self.aliases.values().filter(|a| self.is_enabled(a.tool)).map(|a| a.task_type.to_owned()));
        types.sort();
        types
    }

    /// Todos los descriptores con su estado de habilitación.
    #[must_use]
    pub fn descriptors(&self) -> Vec<(&ToolDescriptor, bool)> {
        self.tools.values().map(|t| (t.descriptor(), self.is_enabled(t.descriptor().name))).collect()
    }

    #[must_use]
    pub fn enabled_count(&self) -> usize {
        self.enabled.len()
    }
}
