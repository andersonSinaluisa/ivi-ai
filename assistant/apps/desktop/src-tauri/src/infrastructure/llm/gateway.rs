//! Inference Gateway (ADR-014 del Cloud Core): el agente habla con un único `LlmProvider` y el gateway decide qué
//! servidor responde, sin cambiar el agente. En esta PC no corre ningún modelo.
//!
//! - `server` (por defecto): el Ollama de tu servidor, publicado por el Cloud Core en `/ollama` y autenticado con la
//!   credencial de este equipo.
//! - `custom`: otro proveedor (Ollama o API compatible) con su URL y su clave.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{ChatRequest, ChatResponse, DeltaSink, LlmError, LlmProvider, ProviderHealth};
use crate::domain::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InferenceMode {
    /// Ollama del servidor emparejado. Acepta los valores antiguos `local` y `auto`.
    #[default]
    #[serde(alias = "local", alias = "auto")]
    Server,
    /// Proveedor propio. Acepta el valor antiguo `remote`.
    #[serde(alias = "remote")]
    Custom,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub mode: InferenceMode,
    pub custom_configured: bool,
    pub custom_name: Option<String>,
    pub last_provider: Option<String>,
}

pub struct InferenceGateway {
    server: Arc<dyn LlmProvider>,
    custom: RwLock<Option<Arc<dyn LlmProvider>>>,
    mode: RwLock<InferenceMode>,
    last: RwLock<Option<String>>,
}

impl InferenceGateway {
    #[must_use]
    pub fn new(server: Arc<dyn LlmProvider>) -> Self {
        Self { server, custom: RwLock::new(None), mode: RwLock::new(InferenceMode::Server), last: RwLock::new(None) }
    }

    pub fn configure(&self, mode: InferenceMode, custom: Option<Arc<dyn LlmProvider>>) {
        if let Ok(mut m) = self.mode.write() {
            *m = mode;
        }
        if let Ok(mut c) = self.custom.write() {
            *c = custom;
        }
    }

    fn mode(&self) -> InferenceMode {
        self.mode.read().map(|m| *m).unwrap_or_default()
    }

    fn custom(&self) -> Option<Arc<dyn LlmProvider>> {
        self.custom.read().ok().and_then(|c| c.clone())
    }

    fn remember(&self, provider: &dyn LlmProvider) {
        if let Ok(mut l) = self.last.write() {
            *l = Some(format!("{} ({})", provider.name(), provider.model()));
        }
    }

    #[must_use]
    pub fn status(&self) -> GatewayStatus {
        let custom = self.custom();
        GatewayStatus {
            mode: self.mode(),
            custom_configured: custom.is_some(),
            custom_name: custom.map(|c| format!("{} ({})", c.name(), c.model())),
            last_provider: self.last.read().map(|l| l.clone()).unwrap_or_default(),
        }
    }

    fn current(&self) -> Result<Arc<dyn LlmProvider>, LlmError> {
        match self.mode() {
            InferenceMode::Server => Ok(self.server.clone()),
            InferenceMode::Custom => {
                self.custom().ok_or_else(|| LlmError::Unavailable("no hay un proveedor propio configurado".into()))
            }
        }
    }
}

#[async_trait]
impl LlmProvider for InferenceGateway {
    fn name(&self) -> &str {
        "gateway"
    }

    fn model(&self) -> String {
        self.current().map_or_else(|_| self.server.model(), |p| p.model())
    }

    async fn health(&self) -> Result<ProviderHealth, LlmError> {
        let provider = self.current()?;
        let h = provider.health().await?;
        Ok(ProviderHealth { version: format!("{}: {}", provider.name(), h.version) })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.current()?.list_models().await
    }

    async fn chat_stream(&self, request: ChatRequest, on_delta: DeltaSink<'_>, cancel: &CancellationToken) -> Result<ChatResponse, LlmError> {
        let provider = self.current()?;
        let response = provider.chat_stream(request, on_delta, cancel).await?;
        self.remember(provider.as_ref());
        Ok(response)
    }

    async fn chat_structured(&self, messages: Vec<ChatMessage>, schema: Value) -> Result<Value, LlmError> {
        let provider = self.current()?;
        let value = provider.chat_structured(messages, schema).await?;
        self.remember(provider.as_ref());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::llm::ollama::{OllamaOptions, OllamaProvider};
    use crate::infrastructure::llm::openai_compat::{OpenAiCompatOptions, OpenAiCompatProvider};
    use crate::infrastructure::llm::Auth;
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn server_provider(base: &str, token: Option<&str>) -> Arc<dyn LlmProvider> {
        Arc::new(
            OllamaProvider::new(OllamaOptions {
                base_url: base.into(),
                model: "qwen3:4b".into(),
                temperature: 0.0,
                num_ctx: 4096,
                think: false,
                request_timeout: Duration::from_secs(5),
                auth: Auth::fixed(token.map(str::to_owned)),
            })
            .expect("server"),
        )
    }

    async fn custom_server(text: &str) -> (MockServer, Arc<dyn LlmProvider>) {
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
                model: "propio".into(),
                api_key: None,
                temperature: 0.0,
                request_timeout: Duration::from_secs(5),
            })
            .expect("custom"),
        );
        (server, provider)
    }

    fn ask() -> ChatRequest {
        ChatRequest { messages: vec![ChatMessage::user("hola")], tools: vec![] }
    }

    #[tokio::test]
    async fn server_mode_sends_the_device_credential_to_the_server_ollama() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ollama/api/chat"))
            .and(header("authorization", "Bearer jwt-del-device"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!("{}\n", json!({"message":{"role":"assistant","content":"hola desde el servidor"},"done":true,"done_reason":"stop"})),
                "application/x-ndjson",
            ))
            .mount(&server)
            .await;
        let gateway = InferenceGateway::new(server_provider(&format!("{}/ollama", server.uri()), Some("jwt-del-device")));
        let r = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect("chat");
        assert_eq!(r.content, "hola desde el servidor");
        assert!(gateway.status().last_provider.expect("último").starts_with("ollama"));
    }

    #[tokio::test]
    async fn unpaired_or_rejected_devices_get_a_clear_error() {
        let gateway = InferenceGateway::new(server_provider("", None));
        let e = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect_err("sin servidor");
        assert!(matches!(&e, LlmError::Unavailable(m) if m.contains("Infraestructura")), "{e}");

        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/ollama/api/chat")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let gateway = InferenceGateway::new(server_provider(&format!("{}/ollama", server.uri()), Some("revocado")));
        let e = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect_err("401");
        assert!(matches!(&e, LlmError::Unavailable(m) if m.contains("credencial")), "{e}");
    }

    #[tokio::test]
    async fn custom_mode_uses_only_the_custom_provider() {
        let (_server, custom) = custom_server("respuesta propia").await;
        let gateway = InferenceGateway::new(server_provider("http://127.0.0.1:9", None));
        gateway.configure(InferenceMode::Custom, None);
        assert!(matches!(gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await, Err(LlmError::Unavailable(_))));
        gateway.configure(InferenceMode::Custom, Some(custom));
        let r = gateway.chat_stream(ask(), &mut |_| {}, &CancellationToken::new()).await.expect("propio");
        assert_eq!(r.content, "respuesta propia");
        assert_eq!(gateway.model(), "propio");
        gateway.configure(InferenceMode::Server, None);
        assert_eq!(gateway.model(), "qwen3:4b");
    }

    #[test]
    fn old_mode_values_still_load() {
        for (old, new) in [("\"local\"", InferenceMode::Server), ("\"auto\"", InferenceMode::Server), ("\"remote\"", InferenceMode::Custom)] {
            assert_eq!(serde_json::from_str::<InferenceMode>(old).expect("modo"), new);
        }
        assert_eq!(serde_json::to_string(&InferenceMode::Server).expect("json"), "\"server\"");
    }
}
