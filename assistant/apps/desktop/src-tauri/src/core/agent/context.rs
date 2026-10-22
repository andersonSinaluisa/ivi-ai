//! OBSERVE y UNDERSTAND: qué hay disponible ahora y qué contexto recibe el modelo.

use std::sync::Arc;

use chrono::Local;

use super::personality::Personality;
use crate::domain::{ChatMessage, ToolSource};
use crate::tools::ToolRegistry;

#[derive(Debug, Clone)]
pub struct Observation {
    pub local_time: String,
    pub os: String,
    pub local_tools: usize,
    pub windows_agent_tools: usize,
}

impl Observation {
    #[must_use]
    pub fn summary(&self) -> String {
        let agent = if self.windows_agent_tools > 0 {
            format!("conectado ({} herramientas)", self.windows_agent_tools)
        } else {
            "no conectado: no puedes abrir apps, leer archivos ni controlar ventanas".into()
        };
        format!(
            "Fecha y hora local: {}. Sistema: {}. Herramientas locales: {}. windows-agent: {}.",
            self.local_time, self.os, self.local_tools, agent
        )
    }
}

pub trait Observer: Send + Sync {
    fn observe(&self) -> Observation;
}

pub struct DefaultObserver {
    registry: Arc<ToolRegistry>,
}

impl DefaultObserver {
    #[must_use]
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

impl Observer for DefaultObserver {
    fn observe(&self) -> Observation {
        Observation {
            local_time: Local::now().format("%Y-%m-%d %H:%M (%:z)").to_string(),
            os: std::env::consts::OS.to_owned(),
            local_tools: self.registry.count(ToolSource::Local),
            windows_agent_tools: self.registry.count(ToolSource::WindowsAgent),
        }
    }
}

/// Construye el contexto del modelo: un único mensaje `system` (personalidad + reglas +
/// observación + recuerdos), el historial reciente y la instrucción.
pub struct ContextBuilder;

impl ContextBuilder {
    /// Orden del mensaje de sistema: personalidad → contexto → recuerdos (datos) → reglas de
    /// seguridad. Las reglas van al final para que ningún recuerdo recuperado pueda "redefinirlas".
    #[must_use]
    pub fn build(
        personality: &Personality,
        observation: &Observation,
        memory_context: &str,
        wants_save: bool,
        history: Vec<ChatMessage>,
        instruction: &str,
    ) -> Vec<ChatMessage> {
        let mut system = personality.persona_prompt();
        system.push_str("\n\nContexto actual: ");
        system.push_str(&observation.summary());
        if !memory_context.is_empty() {
            system.push_str("\n\n");
            system.push_str(memory_context);
        }
        if wants_save {
            system.push_str("\n\nEl usuario quiere que recuerdes algo: guárdalo con la herramienta save_memory y confirma qué guardaste.");
        }
        system.push_str("\n\n");
        system.push_str(super::personality::SAFETY_RULES);
        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(ChatMessage::system(system));
        messages.extend(history.into_iter().filter(|m| m.role != crate::domain::Role::System));
        messages.push(ChatMessage::user(instruction));
        messages
    }
}
