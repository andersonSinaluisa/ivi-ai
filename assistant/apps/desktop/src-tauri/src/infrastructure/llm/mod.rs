//! Abstracción `LlmProvider`: el resto del sistema no sabe qué modelo ni qué servidor hay detrás.

pub mod gateway;
pub mod ollama;
pub mod openai_compat;
#[cfg(test)]
pub mod scripted;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::domain::{ChatMessage, ToolCall};

/// Herramienta tal como se le describe al modelo.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<LlmTool>,
}

/// Fragmento recibido en streaming.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmDelta {
    Content(String),
    Thinking(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub done_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderHealth {
    pub version: String,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum LlmError {
    /// No se pudo conectar (servicio apagado o no instalado).
    #[error("el proveedor de LLM no responde: {0}")]
    Unavailable(String),
    #[error("el modelo '{0}' no está instalado")]
    ModelNotFound(String),
    #[error("error HTTP {status}: {body}")]
    Http { status: u16, body: String },
    /// Respuesta con forma inesperada.
    #[error("respuesta inválida del proveedor: {0}")]
    Protocol(String),
    #[error("tiempo de espera agotado")]
    Timeout,
    #[error("cancelado")]
    Cancelled,
}

pub type DeltaSink<'a> = &'a mut (dyn FnMut(LlmDelta) + Send);

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self) -> String;

    async fn health(&self) -> Result<ProviderHealth, LlmError>;

    async fn list_models(&self) -> Result<Vec<String>, LlmError>;

    /// Chat con herramientas y respuesta en streaming. Los fragmentos de texto llegan a `on_delta`;
    /// el resultado final trae el texto completo y las llamadas a herramientas propuestas.
    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: DeltaSink<'_>,
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, LlmError>;

    /// Salida estructurada: el modelo debe devolver JSON que cumpla `schema`.
    async fn chat_structured(&self, messages: Vec<ChatMessage>, schema: Value) -> Result<Value, LlmError>;
}
