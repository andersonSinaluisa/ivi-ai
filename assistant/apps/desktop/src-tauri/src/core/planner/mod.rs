//! PLAN: decide el siguiente paso. En esta versión el plan lo produce el LLM por tool calling,
//! una decisión por vuelta (respuesta final o llamadas a herramientas).

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::domain::{ChatMessage, ToolCall};
use crate::infrastructure::llm::{ChatRequest, LlmDelta, LlmError, LlmProvider, LlmTool};

#[derive(Debug, Clone, PartialEq)]
pub enum PlanDecision {
    /// Respuesta final para el usuario.
    Answer(String),
    /// Pasos a ejecutar; `preamble` es el texto que el modelo escribió antes de pedirlos.
    Act { preamble: String, calls: Vec<ToolCall> },
}

#[async_trait]
pub trait Planner: Send + Sync {
    async fn next(
        &self,
        messages: &[ChatMessage],
        tools: Vec<LlmTool>,
        on_token: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<PlanDecision, LlmError>;
}

pub struct LlmPlanner {
    provider: Arc<dyn LlmProvider>,
}

impl LlmPlanner {
    #[must_use]
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

/// Algunos modelos Qwen dejan bloques `<think>…</think>` en el contenido.
#[must_use]
pub fn strip_think(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

#[async_trait]
impl Planner for LlmPlanner {
    async fn next(
        &self,
        messages: &[ChatMessage],
        tools: Vec<LlmTool>,
        on_token: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<PlanDecision, LlmError> {
        let mut sink = |d: LlmDelta| {
            if let LlmDelta::Content(t) = d {
                on_token(t);
            }
        };
        let response = self
            .provider
            .chat_stream(ChatRequest { messages: messages.to_vec(), tools }, &mut sink, cancel)
            .await?;
        let text = strip_think(&response.content);
        if response.tool_calls.is_empty() {
            Ok(PlanDecision::Answer(text))
        } else {
            Ok(PlanDecision::Act { preamble: text, calls: response.tool_calls })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_think;

    #[test]
    fn removes_think_blocks() {
        assert_eq!(strip_think("<think>razono</think>\nHola"), "Hola");
        assert_eq!(strip_think("A<think>x</think>B<think>y"), "AB");
        assert_eq!(strip_think("sin bloques"), "sin bloques");
    }
}
