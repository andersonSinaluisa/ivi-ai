//! Doble de pruebas: un proveedor que devuelve respuestas guionizadas.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{ChatRequest, ChatResponse, DeltaSink, LlmDelta, LlmError, LlmProvider, ProviderHealth};
use crate::domain::ChatMessage;

pub enum Scripted {
    Reply(ChatResponse),
    Fail(LlmError),
    /// Espera hasta que se cancele.
    Hang,
}

#[derive(Default)]
pub struct ScriptedProvider {
    script: Mutex<VecDeque<Scripted>>,
    pub requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    #[must_use]
    pub fn new(script: Vec<Scripted>) -> Self {
        Self { script: Mutex::new(script.into()), requests: Mutex::new(Vec::new()) }
    }

    #[must_use]
    pub fn text(s: &str) -> Scripted {
        Scripted::Reply(ChatResponse { content: s.to_owned(), tool_calls: vec![], done_reason: Some("stop".into()) })
    }

    #[must_use]
    pub fn calls(calls: Vec<crate::domain::ToolCall>) -> Scripted {
        Scripted::Reply(ChatResponse { content: String::new(), tool_calls: calls, done_reason: Some("stop".into()) })
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn model(&self) -> String {
        "scripted".into()
    }

    async fn health(&self) -> Result<ProviderHealth, LlmError> {
        Ok(ProviderHealth { version: "test".into() })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(vec!["scripted".into()])
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: DeltaSink<'_>,
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, LlmError> {
        self.requests.lock().expect("lock").push(request);
        let next = self.script.lock().expect("lock").pop_front();
        match next {
            Some(Scripted::Reply(r)) => {
                if !r.content.is_empty() {
                    on_delta(LlmDelta::Content(r.content.clone()));
                }
                Ok(r)
            }
            Some(Scripted::Fail(e)) => Err(e),
            Some(Scripted::Hang) => {
                tokio::select! {
                    () = cancel.cancelled() => Err(LlmError::Cancelled),
                    () = tokio::time::sleep(Duration::from_secs(30)) => Err(LlmError::Timeout),
                }
            }
            None => Err(LlmError::Protocol("guion agotado".into())),
        }
    }

    async fn chat_structured(&self, _messages: Vec<ChatMessage>, _schema: Value) -> Result<Value, LlmError> {
        Err(LlmError::Protocol("no soportado en el doble".into()))
    }
}
