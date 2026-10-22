//! Inference Gateway (ADR-014 del Cloud Core): el agente habla con un único `LlmProvider` y el gateway decide si
//! el modelo corre en esta PC (Ollama local), en un Ollama remoto o en una API compatible, sin cambiar el agente.
//!
//! - `local`: solo Ollama local.
//! - `remote`: solo el proveedor remoto configurado.
//! - `auto`: local; si no responde (servicio apagado o modelo no instalado) y aún no se envió texto, el remoto.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{ChatRequest, ChatResponse, DeltaSink, LlmDelta, LlmError, LlmProvider, ProviderHealth};
use crate::domain::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InferenceMode {
    #[default]
    Local,
    Remote,
    Auto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub mode: InferenceMode,
    pub remote_configured: bool,
    pub remote_name: Option<String>,
    pub last_provider: Option<String>,
    pub last_fallback: Option<String>,
}

pub struct InferenceGateway {
    local: Arc<dyn LlmProvider>,
    remote: RwLock<Option<Arc<dyn LlmProvider>>>,
    mode: RwLock<InferenceMode>,
    last: RwLock<(Option<String>, Option<String>)>,
}

fn can_fall_back(e: &LlmError) -> bool {
    matches!(e, LlmError::Unavailable(_) | LlmError::ModelNotFound(_))
}

impl InferenceGateway {
    #[must_use]
    pub fn new(local: Arc<dyn LlmProvider>) -> Self {
        Self { local, remote: RwLock::new(None), mode: RwLock::new(InferenceMode::Local), last: RwLock::new((None, None)) }
    }

    pub fn configure(&self, mode: InferenceMode, remote: Option<Arc<dyn LlmProvider>>) {
        if let Ok(mut m) = self.mode.write() {
            *m = mode;
        }
        if let Ok(mut r) = self.remote.write() {
            *r = remote;
        }
    }

    fn mode(&self) -> InferenceMode {
        self.mode.read().map(|m| *m).unwrap_or_default()
    }

    fn remote(&self) -> Option<Arc<dyn LlmProvider>> {
        self.remote.read().ok().and_then(|r| r.clone())
    }

    fn remember(&self, provider: &dyn LlmProvider, fallback: Option<String>) {
        if let Ok(mut l) = self.last.write() {
            *l = (Some(format!("{} ({})", provider.name(), provider.model())), fallback);
        }
    }

    #[must_use]
    pub fn status(&self) -> GatewayStatus {
        let remote = self.remote();
        let (last_provider, last_fallback) = self.last.read().map(|l| l.clone()).unwrap_or_default();
        GatewayStatus {
            mode: self.mode(),
            remote_configured: remote.is_some(),
            remote_name: remote.map(|r| format!("{} ({})", r.name(), r.model())),
            last_provider,
            last_fallback,
        }
    }

    /// Orden de intento según el modo.
    fn chain(&self) -> Result<Vec<Arc<dyn LlmProvider>>, LlmError> {
        match (self.mode(), self.remote()) {
            (InferenceMode::Local, _) => Ok(vec![self.local.clone()]),
            (InferenceMode::Remote, Some(remote)) => Ok(vec![remote]),
            (InferenceMode::Remote, None) => Err(LlmError::Unavailable("no hay un proveedor remoto configurado".into())),
            (InferenceMode::Auto, Some(remote)) => Ok(vec![self.local.clone(), remote]),
            (InferenceMode::Auto, None) => Ok(vec![self.local.clone()]),
        }
    }
}

#[async_trait]
impl LlmProvider for InferenceGateway {
    fn name(&self) -> &str {
        "gateway"
    }

    fn model(&self) -> String {
        match (self.mode(), self.remote()) {
            (InferenceMode::Remote, Some(r)) => r.model(),
            _ => self.local.model(),
        }
    }

    async fn health(&self) -> Result<ProviderHealth, LlmError> {
        let mut last = LlmError::Unavailable("sin proveedores".into());
        for provider in self.chain()? {
            match provider.health().await {
                Ok(h) => return Ok(ProviderHealth { version: format!("{}: {}", provider.name(), h.version) }),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let chain = self.chain()?;
        chain[0].list_models().await
    }

    async fn chat_stream(&self, request: ChatRequest, on_delta: DeltaSink<'_>, cancel: &CancellationToken) -> Result<ChatResponse, LlmError> {
        let chain = self.chain()?;
        let mut fallback = None;
        let mut last = LlmError::Unavailable("sin proveedores".into());
        for (i, provider) in chain.iter().enumerate() {
            let mut streamed = false;
            let mut forward = |d: LlmDelta| {
                streamed = true;
                on_delta(d);
            };
            match provider.chat_stream(request.clone(), &mut forward, cancel).await {
                Ok(response) => {
                    self.remember(provider.as_ref(), fallback);
                    return Ok(response);
                }
                Err(e) if i + 1 < chain.len() && !streamed && can_fall_back(&e) => {
                    tracing::warn!(error = %e, "modelo local no disponible; se usa el proveedor remoto");
                    fallback = Some(e.to_string());
                    last = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    async fn chat_structured(&self, messages: Vec<ChatMessage>, schema: Value) -> Result<Value, LlmError> {
        let chain = self.chain()?;
        let mut last = LlmError::Unavailable("sin proveedores".into());
        for (i, provider) in chain.iter().enumerate() {
            match provider.chat_structured(messages.clone(), schema.clone()).await {
                Ok(v) => {
                    self.remember(provider.as_ref(), None);
                    return Ok(v);
                }
                Err(e) if i + 1 < chain.len() && can_fall_back(&e) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::llm::ollama::{OllamaOptions, OllamaProvider};
    use crate::infrastructure::llm::openai_compat::{OpenAiCompatOptions, OpenAiCompatProvider};
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn local(base: &str) -> Arc<dyn LlmProvider> {
        Arc::new(
            OllamaProvider::new(OllamaOptions {
                base_url: base.into(),
                model: "qwen3:4b".into(),
                temperature: 0.0,
                num_ctx: 4096,
                think: false,
                request_timeout: Duration::from_secs(5),
                api_key: None,
            })
            .expect("local"),
        )
    }

    async fn remote_server(text: &str) -> (MockServer, Arc<dyn LlmProvider>) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"delta":{"content": text},"finish_reason":"stop"}]})),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let provider: Arc<dyn LlmProvider> = Arc::new(
            OpenAiCompatProvider::new(OpenAiCompatOptions {
                base_url: format!("{}/v1", server.uri()),
                model: "remoto".into(),
                api_key: None,
                temperature: 0.0,
                request_timeout: Duration::from_secs(5),
            })
            .expect("remote"),
        );
        (server, provider)
    }

    fn ask() -> ChatRequest {
        ChatRequest { messages: vec![ChatMessage::user("hola")], tools: vec![] }
    }

    #[tokio::test]
    async fn auto_falls_back_to_the_remote_provider_when_ollama_is_down() {
        let (_server, remote) = remote_server("respuesta remota").await;
        let gateway = InferenceGateway::new(local("http://127.0.0.1:9"));
        gateway.configure(InferenceMode::Auto, Some(remote));
        let r = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect("chat");
        assert_eq!(r.content, "respuesta remota");
        let status = gateway.status();
        assert!(status.last_provider.expect("último").starts_with("openai-compatible"));
        assert!(status.last_fallback.is_some());
    }

    #[tokio::test]
    async fn local_mode_never_uses_the_remote_and_remote_mode_needs_one() {
        let (_server, remote) = remote_server("no debería usarse").await;
        let gateway = InferenceGateway::new(local("http://127.0.0.1:9"));
        gateway.configure(InferenceMode::Local, Some(remote.clone()));
        assert!(matches!(gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await, Err(LlmError::Unavailable(_))));

        gateway.configure(InferenceMode::Remote, None);
        assert!(matches!(gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await, Err(LlmError::Unavailable(_))));

        gateway.configure(InferenceMode::Remote, Some(remote));
        let r = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect("remoto");
        assert_eq!(r.content, "no debería usarse");
        assert_eq!(gateway.model(), "remoto");
    }

    #[tokio::test]
    async fn other_errors_do_not_trigger_a_fallback() {
        let local_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({"error": "falló el modelo"})))
            .mount(&local_server)
            .await;
        let (_server, remote) = remote_server("x").await;
        let gateway = InferenceGateway::new(local(&local_server.uri()));
        gateway.configure(InferenceMode::Auto, Some(remote));
        let e = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect_err("500");
        assert!(matches!(e, LlmError::Http { status: 500, .. }));
    }
}
