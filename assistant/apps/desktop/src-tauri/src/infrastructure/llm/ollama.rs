//! Cliente de Ollama sobre la API nativa `/api/chat` (no la compatible con OpenAI).
//!
//! - Streaming NDJSON: una línea JSON por fragmento.
//! - Tool calling: `tools` en la petición, `message.tool_calls` en la respuesta.
//! - Un único mensaje `system` al principio (algunas versiones de Ollama rechazan más de uno).

use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{Auth, ChatRequest, ChatResponse, DeltaSink, LlmDelta, LlmError, LlmProvider, ProviderHealth};
use crate::domain::{ChatMessage, Role, ToolCall};

#[derive(Debug, Clone)]
pub struct OllamaOptions {
    pub base_url: String,
    pub model: String,
    pub temperature: f32,
    pub num_ctx: u32,
    pub think: bool,
    pub request_timeout: Duration,
    /// Bearer para el Ollama del servidor (credencial del device) o un proxy propio.
    pub auth: Auth,
}

pub struct OllamaProvider {
    client: reqwest::Client,
    options: RwLock<OllamaOptions>,
}

impl OllamaProvider {
    /// # Errors
    /// Si no se puede construir el cliente HTTP.
    pub fn new(options: OllamaOptions) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| LlmError::Protocol(e.to_string()))?;
        Ok(Self { client, options: RwLock::new(options) })
    }

    #[must_use]
    pub fn options(&self) -> OllamaOptions {
        self.options.read().map(|o| o.clone()).unwrap_or_else(|p| p.into_inner().clone())
    }

    pub fn set_options(&self, options: OllamaOptions) {
        match self.options.write() {
            Ok(mut guard) => *guard = options,
            Err(poisoned) => *poisoned.into_inner() = options,
        }
    }

    fn url(&self, path: &str) -> Result<String, LlmError> {
        let base = self.options().base_url;
        if base.trim().is_empty() {
            return Err(LlmError::Unavailable(
                "este equipo no está conectado a tu servidor. Empareja la PC en Configuración → Infraestructura".into(),
            ));
        }
        Ok(format!("{}{path}", base.trim_end_matches('/')))
    }

    fn body(&self, request: &ChatRequest, stream: bool) -> Value {
        let o = self.options();
        let mut body = json!({
            "model": o.model,
            "messages": encode_messages(&request.messages),
            "stream": stream,
            "think": o.think,
            "options": { "temperature": o.temperature, "num_ctx": o.num_ctx },
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "type": "function",
                            "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                        })
                    })
                    .collect(),
            );
        }
        body
    }

    async fn post(&self, path: &str, body: &Value) -> Result<reqwest::Response, LlmError> {
        let token = self.options().auth.token().await.map_err(credential_error)?;
        let response = self
            .client
            .post(self.url(path)?)
            .timeout(self.options().request_timeout)
            .bearer_auth_opt(token.as_deref())
            .json(body)
            .send()
            .await
            .map_err(map_transport)?;
        check_status(response, &self.options().model).await
    }
}

/// `bearer_auth` solo si hay token.
pub(crate) trait BearerOpt {
    fn bearer_auth_opt(self, token: Option<&str>) -> Self;
}

impl BearerOpt for reqwest::RequestBuilder {
    fn bearer_auth_opt(self, token: Option<&str>) -> Self {
        match token.filter(|t| !t.is_empty()) {
            Some(t) => self.bearer_auth(t),
            None => self,
        }
    }
}

fn credential_error(reason: String) -> LlmError {
    LlmError::Unavailable(format!("este equipo no tiene credencial para tu servidor ({reason})"))
}

fn map_transport(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout
    } else if e.is_connect() {
        LlmError::Unavailable(format!("no se pudo conectar con el servidor de modelos ({e}). ¿Hay Internet?"))
    } else {
        LlmError::Unavailable(e.to_string())
    }
}

async fn check_status(response: reqwest::Response, model: &str) -> Result<reqwest::Response, LlmError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let ollama_error = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned));
    // Un 401 de Ollama (JSON con `error`) no es la credencial del equipo: las versiones recientes lo devuelven al
    // pedir un modelo que no está descargado (lo buscan en ollama.com). El 401 de nginx no trae JSON.
    if let (401 | 403, Some(e)) = (status.as_u16(), ollama_error.as_deref()) {
        return Err(LlmError::Unavailable(format!(
            "Ollama del servidor respondió «{e}» para el modelo '{model}'. ¿Está descargado? En el servidor: ollama list / ollama pull {model}"
        )));
    }
    let message = ollama_error.unwrap_or(body);
    if status.as_u16() == 404 && message.contains("not found") {
        return Err(LlmError::ModelNotFound(model.to_owned()));
    }
    match status.as_u16() {
        401 | 403 => {
            return Err(LlmError::Unavailable(
                "el servidor rechazó la credencial de este equipo: vuelve a emparejarlo en Configuración → Infraestructura".into(),
            ))
        }
        502..=504 => return Err(LlmError::Unavailable(format!("Ollama no responde en el servidor (HTTP {status})"))),
        _ => {}
    }
    Err(LlmError::Http { status: status.as_u16(), body: message })
}

/// Convierte el historial al formato de Ollama, fusionando los mensajes `system` en uno solo.
#[must_use]
pub fn encode_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let system: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::System).map(|m| m.content.as_str()).collect();
    let mut out = Vec::with_capacity(messages.len());
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system.join("\n\n") }));
    }
    for m in messages.iter().filter(|m| m.role != Role::System) {
        let mut v = json!({ "role": m.role.as_str(), "content": m.content });
        if !m.tool_calls.is_empty() {
            v["tool_calls"] = Value::Array(
                m.tool_calls
                    .iter()
                    .map(|c| json!({ "function": { "name": c.name, "arguments": c.arguments } }))
                    .collect(),
            );
        }
        if let Some(name) = &m.tool_name {
            v["tool_name"] = Value::String(name.clone());
        }
        out.push(v);
    }
    out
}

/// Extrae las llamadas a herramientas de un `message` de Ollama.
fn decode_tool_calls(message: &Value, next_id: &mut usize) -> Result<Vec<ToolCall>, LlmError> {
    let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(calls.len());
    for call in calls {
        let function = call.get("function").ok_or_else(|| LlmError::Protocol("tool_call sin 'function'".into()))?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| LlmError::Protocol("tool_call sin nombre".into()))?;
        // Algunos modelos devuelven los argumentos como string JSON.
        let arguments = match function.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(Value::String(s)) => serde_json::from_str(s)
                .map_err(|e| LlmError::Protocol(format!("argumentos inválidos para '{name}': {e}")))?,
            Some(v) => v.clone(),
        };
        *next_id += 1;
        let id = call.get("id").and_then(Value::as_str).map_or_else(|| format!("call_{next_id}"), str::to_owned);
        out.push(ToolCall { id, name: name.to_owned(), arguments });
    }
    Ok(out)
}

/// Procesa una línea NDJSON. Devuelve `true` cuando llega `done`.
fn process_line(
    line: &str,
    acc: &mut ChatResponse,
    next_id: &mut usize,
    on_delta: &mut (dyn FnMut(LlmDelta) + Send),
) -> Result<bool, LlmError> {
    let v: Value = serde_json::from_str(line).map_err(|e| LlmError::Protocol(format!("línea NDJSON inválida: {e}")))?;
    if let Some(error) = v.get("error").and_then(Value::as_str) {
        return Err(LlmError::Http { status: 500, body: error.to_owned() });
    }
    if let Some(message) = v.get("message") {
        if let Some(thinking) = message.get("thinking").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            on_delta(LlmDelta::Thinking(thinking.to_owned()));
        }
        if let Some(content) = message.get("content").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            acc.content.push_str(content);
            on_delta(LlmDelta::Content(content.to_owned()));
        }
        acc.tool_calls.extend(decode_tool_calls(message, next_id)?);
    }
    let done = v.get("done").and_then(Value::as_bool).unwrap_or(false);
    if done {
        acc.done_reason = v.get("done_reason").and_then(Value::as_str).map(str::to_owned);
    }
    Ok(done)
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn model(&self) -> String {
        self.options().model
    }

    async fn health(&self) -> Result<ProviderHealth, LlmError> {
        let token = self.options().auth.token().await.map_err(credential_error)?;
        let response = self
            .client
            .get(self.url("/api/version")?)
            .timeout(Duration::from_secs(3))
            .bearer_auth_opt(token.as_deref())
            .send()
            .await
            .map_err(map_transport)?;
        let response = check_status(response, &self.model()).await?;
        let v: Value = response.json().await.map_err(|e| LlmError::Protocol(e.to_string()))?;
        Ok(ProviderHealth { version: v.get("version").and_then(Value::as_str).unwrap_or("desconocida").to_owned() })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let token = self.options().auth.token().await.map_err(credential_error)?;
        let response = self
            .client
            .get(self.url("/api/tags")?)
            .timeout(Duration::from_secs(5))
            .bearer_auth_opt(token.as_deref())
            .send()
            .await
            .map_err(map_transport)?;
        let response = check_status(response, &self.model()).await?;
        let v: Value = response.json().await.map_err(|e| LlmError::Protocol(e.to_string()))?;
        let models = v
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| LlmError::Protocol("falta 'models' en /api/tags".into()))?;
        Ok(models.iter().filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_owned)).collect())
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: DeltaSink<'_>,
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, LlmError> {
        let body = self.body(&request, true);
        let response = tokio::select! {
            () = cancel.cancelled() => return Err(LlmError::Cancelled),
            r = self.post("/api/chat", &body) => r?,
        };

        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        let mut acc = ChatResponse { content: String::new(), tool_calls: Vec::new(), done_reason: None };
        let mut next_id = 0usize;

        loop {
            let chunk = tokio::select! {
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| if e.is_timeout() { LlmError::Timeout } else { LlmError::Protocol(e.to_string()) })?;
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if process_line(line, &mut acc, &mut next_id, on_delta)? {
                    return Ok(acc);
                }
            }
        }
        // El servidor cerró sin `done: true`: se procesa lo que quede.
        let rest = String::from_utf8_lossy(&buffer).trim().to_owned();
        if !rest.is_empty() && process_line(&rest, &mut acc, &mut next_id, on_delta)? {
            return Ok(acc);
        }
        Err(LlmError::Protocol("la respuesta terminó sin 'done'".into()))
    }

    async fn chat_structured(&self, messages: Vec<ChatMessage>, schema: Value) -> Result<Value, LlmError> {
        let mut body = self.body(&ChatRequest { messages, tools: Vec::new() }, false);
        body["format"] = schema;
        let response = self.post("/api/chat", &body).await?;
        let v: Value = response.json().await.map_err(|e| LlmError::Protocol(e.to_string()))?;
        let content = v
            .pointer("/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| LlmError::Protocol("falta message.content".into()))?;
        serde_json::from_str(content).map_err(|e| LlmError::Protocol(format!("el modelo no devolvió JSON válido: {e}")))
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_401_from_ollama_itself_points_to_the_model_not_the_credential() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"error": "unauthorized"})))
            .mount(&server)
            .await;
        let p = provider(&server.uri());
        let e = p.chat_structured(vec![crate::domain::ChatMessage::user("hola")], serde_json::json!({})).await.expect_err("401");
        assert!(matches!(&e, LlmError::Unavailable(m) if m.contains("ollama pull") && m.contains("unauthorized")), "{e}");

        let nginx = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(401).set_body_string("<html>401 Authorization Required</html>"))
            .mount(&nginx)
            .await;
        let e = provider(&nginx.uri()).chat_structured(vec![crate::domain::ChatMessage::user("hola")], serde_json::json!({})).await.expect_err("401");
        assert!(matches!(&e, LlmError::Unavailable(m) if m.contains("credencial")), "{e}");
    }

    use super::*;
    use crate::infrastructure::llm::LlmTool;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base_url: &str) -> OllamaProvider {
        OllamaProvider::new(OllamaOptions {
            base_url: base_url.to_owned(),
            model: "qwen3:4b".into(),
            temperature: 0.2,
            num_ctx: 4096,
            think: false,
            request_timeout: Duration::from_secs(5),
            auth: Auth::none(),
        })
        .expect("provider")
    }

    fn ndjson(lines: &[Value]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    #[tokio::test]
    async fn streams_tokens_and_returns_the_full_text() {
        let server = MockServer::start().await;
        let body = ndjson(&[
            json!({"message": {"role": "assistant", "content": "Hola"}, "done": false}),
            json!({"message": {"role": "assistant", "content": ", Anderson."}, "done": false}),
            json!({"message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop"}),
        ]);
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(json!({"model": "qwen3:4b", "stream": true})))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let p = provider(&server.uri());
        let mut deltas = Vec::new();
        let mut sink = |d: LlmDelta| deltas.push(d);
        let request = ChatRequest { messages: vec![ChatMessage::user("hola")], tools: vec![] };
        let r = p.chat_stream(request, &mut sink, &CancellationToken::new()).await.expect("ok");

        assert_eq!(r.content, "Hola, Anderson.");
        assert_eq!(r.done_reason.as_deref(), Some("stop"));
        assert_eq!(deltas, vec![LlmDelta::Content("Hola".into()), LlmDelta::Content(", Anderson.".into())]);
    }

    #[tokio::test]
    async fn parses_tool_calls_and_sends_tool_definitions() {
        let server = MockServer::start().await;
        let body = ndjson(&[
            json!({"message": {"role": "assistant", "content": "", "tool_calls": [
                {"function": {"name": "get_current_time", "arguments": {}}},
                {"function": {"name": "list_directory", "arguments": "{\"path\": \"C:/x\"}"}}
            ]}, "done": false}),
            json!({"message": {"role": "assistant", "content": ""}, "done": true}),
        ]);
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(json!({"tools": [{"type": "function", "function": {"name": "get_current_time"}}]})))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let p = provider(&server.uri());
        let tools = vec![LlmTool {
            name: "get_current_time".into(),
            description: "hora".into(),
            parameters: json!({"type": "object", "properties": {}}),
        }];
        let mut sink = |_d: LlmDelta| {};
        let r = p
            .chat_stream(ChatRequest { messages: vec![ChatMessage::user("hora?")], tools }, &mut sink, &CancellationToken::new())
            .await
            .expect("ok");

        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].name, "get_current_time");
        assert_eq!(r.tool_calls[1].arguments, json!({"path": "C:/x"}), "argumentos en string se decodifican");
        assert_ne!(r.tool_calls[0].id, r.tool_calls[1].id);
    }

    #[tokio::test]
    async fn missing_model_is_reported_as_model_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(404).set_body_json(json!({"error": "model \"qwen3:4b\" not found, try pulling it first"})),
            )
            .mount(&server)
            .await;
        let mut sink = |_d: LlmDelta| {};
        let err = provider(&server.uri())
            .chat_stream(ChatRequest { messages: vec![ChatMessage::user("x")], tools: vec![] }, &mut sink, &CancellationToken::new())
            .await
            .expect_err("debe fallar");
        assert_eq!(err, LlmError::ModelNotFound("qwen3:4b".into()));
    }

    #[tokio::test]
    async fn stream_error_lines_and_malformed_lines_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"error\":\"out of memory\"}\n"))
            .mount(&server)
            .await;
        let mut sink = |_d: LlmDelta| {};
        let err = provider(&server.uri())
            .chat_stream(ChatRequest { messages: vec![ChatMessage::user("x")], tools: vec![] }, &mut sink, &CancellationToken::new())
            .await
            .expect_err("debe fallar");
        assert!(matches!(err, LlmError::Http { ref body, .. } if body == "out of memory"), "{err:?}");

        let server2 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_string("esto no es json\n"))
            .mount(&server2)
            .await;
        let err = provider(&server2.uri())
            .chat_stream(ChatRequest { messages: vec![ChatMessage::user("x")], tools: vec![] }, &mut sink, &CancellationToken::new())
            .await
            .expect_err("debe fallar");
        assert!(matches!(err, LlmError::Protocol(_)));
    }

    #[tokio::test]
    async fn unreachable_server_is_unavailable() {
        // Puerto cerrado en loopback.
        let p = provider("http://127.0.0.1:9");
        let err = p.health().await.expect_err("debe fallar");
        assert!(matches!(err, LlmError::Unavailable(_)), "{err:?}");
    }

    #[tokio::test]
    async fn lists_models_and_reads_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{"name": "qwen3:4b"}, {"name": "qwen3:8b"}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "0.12.0"})))
            .mount(&server)
            .await;
        let p = provider(&server.uri());
        assert_eq!(p.list_models().await.expect("ok"), vec!["qwen3:4b", "qwen3:8b"]);
        assert_eq!(p.health().await.expect("ok").version, "0.12.0");
    }

    #[tokio::test]
    async fn structured_output_sends_format_and_parses_json() {
        let server = MockServer::start().await;
        let schema = json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]});
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(json!({"stream": false, "format": {"type": "object"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": {"content": "{\"ok\": true}"}, "done": true})))
            .mount(&server)
            .await;
        let v = provider(&server.uri()).chat_structured(vec![ChatMessage::user("x")], schema).await.expect("ok");
        assert_eq!(v, json!({"ok": true}));
    }

    #[tokio::test]
    async fn cancellation_stops_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)).set_body_string(""))
            .mount(&server)
            .await;
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            c2.cancel();
        });
        let mut sink = |_d: LlmDelta| {};
        let started = std::time::Instant::now();
        let err = provider(&server.uri())
            .chat_stream(ChatRequest { messages: vec![ChatMessage::user("x")], tools: vec![] }, &mut sink, &cancel)
            .await
            .expect_err("cancelado");
        assert_eq!(err, LlmError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    /// Prueba contra un Ollama real. Se ejecuta solo con `JARVIS_TEST_OLLAMA=1`
    /// (y opcionalmente `JARVIS_TEST_MODEL=qwen3:4b`).
    #[tokio::test]
    async fn real_ollama_round_trip_with_tools() {
        if std::env::var("JARVIS_TEST_OLLAMA").is_err() {
            eprintln!("omitida: define JARVIS_TEST_OLLAMA=1 para probar contra Ollama real");
            return;
        }
        let model = std::env::var("JARVIS_TEST_MODEL").unwrap_or_else(|_| "qwen3:4b".into());
        let p = OllamaProvider::new(OllamaOptions {
            base_url: "http://127.0.0.1:11434".into(),
            model: model.clone(),
            temperature: 0.0,
            num_ctx: 4096,
            think: false,
            request_timeout: Duration::from_secs(180),
            auth: Auth::none(),
        })
        .expect("provider");
        let version = p.health().await.expect("Ollama debe estar corriendo");
        eprintln!("Ollama {}", version.version);
        assert!(p.list_models().await.expect("tags").iter().any(|m| m.starts_with(&model)), "falta {model}");

        let tools = vec![LlmTool {
            name: "get_current_time".into(),
            description: "Devuelve la fecha y hora actuales".into(),
            parameters: json!({"type": "object", "properties": {}}),
        }];
        let mut tokens = 0usize;
        let mut sink = |_d: LlmDelta| tokens += 1;
        let r = p
            .chat_stream(
                ChatRequest {
                    messages: vec![
                        ChatMessage::system("Responde en español. Usa herramientas cuando las necesites."),
                        ChatMessage::user("¿Qué hora es? Usa la herramienta."),
                    ],
                    tools,
                },
                &mut sink,
                &CancellationToken::new(),
            )
            .await
            .expect("chat");
        eprintln!("contenido={:?} tool_calls={:?}", r.content, r.tool_calls);
        assert!(!r.tool_calls.is_empty() || !r.content.is_empty());
    }

    #[test]
    fn system_messages_are_merged_into_one() {
        let msgs = vec![
            ChatMessage::system("a"),
            ChatMessage::user("u"),
            ChatMessage::system("b"),
            ChatMessage::tool("get_current_time", "{}"),
        ];
        let enc = encode_messages(&msgs);
        assert_eq!(enc.len(), 3);
        assert_eq!(enc[0], json!({"role": "system", "content": "a\n\nb"}));
        assert_eq!(enc[2]["tool_name"], "get_current_time");
    }
}
