//! Proveedor para APIs compatibles con el formato *chat completions* (`POST {base}/chat/completions`), con
//! herramientas y streaming SSE. Sirve para un servidor propio (vLLM, LM Studio, llama.cpp server, Ollama en
//! `/v1`) o un servicio externo. La clave, si existe, viene del almacén de secretos del sistema.

use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::ollama::BearerOpt;
use super::{ChatRequest, ChatResponse, DeltaSink, LlmDelta, LlmError, LlmProvider, ProviderHealth};
use crate::domain::{ChatMessage, Role, ToolCall};

#[derive(Debug, Clone)]
pub struct OpenAiCompatOptions {
    /// Base con la versión, p. ej. `https://mi-servidor/v1`.
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub temperature: f32,
    pub request_timeout: Duration,
}

pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    options: RwLock<OpenAiCompatOptions>,
}

impl OpenAiCompatProvider {
    /// # Errors
    /// Si no se puede construir el cliente HTTP.
    pub fn new(options: OpenAiCompatOptions) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| LlmError::Protocol(e.to_string()))?;
        Ok(Self { client, options: RwLock::new(options) })
    }

    fn options(&self) -> OpenAiCompatOptions {
        self.options.read().map(|o| o.clone()).unwrap_or_else(|p| p.into_inner().clone())
    }

    pub fn set_options(&self, options: OpenAiCompatOptions) {
        match self.options.write() {
            Ok(mut g) => *g = options,
            Err(p) => *p.into_inner() = options,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.options().base_url.trim_end_matches('/'))
    }

    fn body(&self, request: &ChatRequest, stream: bool) -> Value {
        let o = self.options();
        let mut body = json!({
            "model": o.model,
            "messages": encode_messages(&request.messages),
            "stream": stream,
            "temperature": o.temperature,
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } }))
                    .collect(),
            );
        }
        body
    }

    async fn post(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let o = self.options();
        let response = self
            .client
            .post(self.url("/chat/completions"))
            .timeout(o.request_timeout)
            .bearer_auth_opt(o.api_key.as_deref())
            .json(body)
            .send()
            .await
            .map_err(transport)?;
        check(response, &o.model).await
    }
}

fn transport(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout
    } else {
        LlmError::Unavailable(e.without_url().to_string())
    }
}

async fn check(response: reqwest::Response, model: &str) -> Result<reqwest::Response, LlmError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or(body);
    if status.as_u16() == 404 && message.to_lowercase().contains("model") {
        return Err(LlmError::ModelNotFound(model.to_owned()));
    }
    Err(LlmError::Http { status: status.as_u16(), body: message.chars().take(500).collect() })
}

/// Historial en formato chat completions. Los resultados de herramientas necesitan el `tool_call_id` de la llamada
/// que responden: se emparejan en orden con las llamadas pendientes del mismo nombre.
#[must_use]
pub fn encode_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let system: Vec<&str> = messages.iter().filter(|m| m.role == Role::System).map(|m| m.content.as_str()).collect();
    let mut out = Vec::with_capacity(messages.len());
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system.join("\n\n") }));
    }
    let mut pending: Vec<(String, String)> = Vec::new();
    for m in messages.iter().filter(|m| m.role != Role::System) {
        match m.role {
            Role::Assistant if !m.tool_calls.is_empty() => {
                pending = m.tool_calls.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
                out.push(json!({
                    "role": "assistant",
                    "content": if m.content.is_empty() { Value::Null } else { Value::String(m.content.clone()) },
                    "tool_calls": m.tool_calls.iter().map(|c| json!({
                        "id": c.id, "type": "function",
                        "function": { "name": c.name, "arguments": c.arguments.to_string() }
                    })).collect::<Vec<_>>(),
                }));
            }
            Role::Tool => {
                let name = m.tool_name.clone().unwrap_or_default();
                let index = pending.iter().position(|(_, n)| *n == name).unwrap_or(0);
                let id = if pending.is_empty() { format!("call_{name}") } else { pending.remove(index).0 };
                out.push(json!({ "role": "tool", "tool_call_id": id, "content": m.content }));
            }
            _ => out.push(json!({ "role": m.role.as_str(), "content": m.content })),
        }
    }
    out
}

/// Llamadas a herramientas que llegan en trozos (índice, nombre, argumentos parciales).
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

fn finish_calls(partials: Vec<PartialCall>) -> Result<Vec<ToolCall>, LlmError> {
    partials
        .into_iter()
        .enumerate()
        .filter(|(_, p)| !p.name.is_empty())
        .map(|(i, p)| {
            let arguments = if p.arguments.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&p.arguments).map_err(|e| LlmError::Protocol(format!("argumentos inválidos para '{}': {e}", p.name)))?
            };
            Ok(ToolCall { id: if p.id.is_empty() { format!("call_{}", i + 1) } else { p.id }, name: p.name, arguments })
        })
        .collect()
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    fn model(&self) -> String {
        self.options().model
    }

    async fn health(&self) -> Result<ProviderHealth, LlmError> {
        let models = self.list_models().await?;
        Ok(ProviderHealth { version: format!("{} modelos", models.len()) })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let o = self.options();
        let response = self
            .client
            .get(self.url("/models"))
            .timeout(Duration::from_secs(5))
            .bearer_auth_opt(o.api_key.as_deref())
            .send()
            .await
            .map_err(transport)?;
        let v: Value = check(response, &o.model).await?.json().await.map_err(|e| LlmError::Protocol(e.to_string()))?;
        Ok(v.get("data")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned)).collect())
            .unwrap_or_default())
    }

    async fn chat_stream(&self, request: ChatRequest, on_delta: DeltaSink<'_>, cancel: &CancellationToken) -> Result<ChatResponse, LlmError> {
        let body = self.body(&request, true);
        let response = tokio::select! {
            () = cancel.cancelled() => return Err(LlmError::Cancelled),
            r = self.post(&body) => r?,
        };
        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        let mut acc = ChatResponse { content: String::new(), tool_calls: Vec::new(), done_reason: None };
        let mut partials: Vec<PartialCall> = Vec::new();
        loop {
            let chunk = tokio::select! {
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            buffer.extend_from_slice(&chunk.map_err(|e| if e.is_timeout() { LlmError::Timeout } else { LlmError::Protocol(e.to_string()) })?);
            while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
                if data == "[DONE]" {
                    acc.tool_calls = finish_calls(std::mem::take(&mut partials))?;
                    return Ok(acc);
                }
                let v: Value = serde_json::from_str(data).map_err(|e| LlmError::Protocol(format!("evento SSE inválido: {e}")))?;
                let Some(choice) = v.pointer("/choices/0") else { continue };
                if let Some(content) = choice.pointer("/delta/content").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                    acc.content.push_str(content);
                    on_delta(LlmDelta::Content(content.to_owned()));
                }
                if let Some(reasoning) = choice.pointer("/delta/reasoning_content").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                    on_delta(LlmDelta::Thinking(reasoning.to_owned()));
                }
                for call in choice.pointer("/delta/tool_calls").and_then(Value::as_array).into_iter().flatten() {
                    let index = call.get("index").and_then(Value::as_u64).map_or(partials.len(), |i| usize::try_from(i).unwrap_or(0));
                    while partials.len() <= index {
                        partials.push(PartialCall::default());
                    }
                    let p = &mut partials[index];
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        p.id = id.to_owned();
                    }
                    if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                        p.name.push_str(name);
                    }
                    if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                        p.arguments.push_str(args);
                    }
                }
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    acc.done_reason = Some(reason.to_owned());
                }
            }
        }
        // Sin [DONE]: algunos servidores cierran la conexión al terminar.
        if acc.done_reason.is_some() {
            acc.tool_calls = finish_calls(partials)?;
            return Ok(acc);
        }
        Err(LlmError::Protocol("la respuesta terminó sin [DONE]".into()))
    }

    async fn chat_structured(&self, messages: Vec<ChatMessage>, schema: Value) -> Result<Value, LlmError> {
        let mut body = self.body(&ChatRequest { messages, tools: Vec::new() }, false);
        body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "salida", "schema": schema } });
        let v: Value = self.post(&body).await?.json().await.map_err(|e| LlmError::Protocol(e.to_string()))?;
        let content = v
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| LlmError::Protocol("falta choices[0].message.content".into()))?;
        serde_json::from_str(content).map_err(|e| LlmError::Protocol(format!("el modelo no devolvió JSON válido: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::llm::LlmTool;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base: &str, key: Option<&str>) -> OpenAiCompatProvider {
        OpenAiCompatProvider::new(OpenAiCompatOptions {
            base_url: format!("{base}/v1"),
            model: "qwen-remoto".into(),
            api_key: key.map(str::to_owned),
            temperature: 0.2,
            request_timeout: Duration::from_secs(5),
        })
        .expect("provider")
    }

    fn sse(events: &[Value]) -> String {
        events.iter().map(|e| format!("data: {e}\n\n")).collect::<String>() + "data: [DONE]\n\n"
    }

    #[tokio::test]
    async fn streams_text_and_assembles_tool_calls_split_across_chunks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer clave-secreta"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                sse(&[
                    json!({"choices":[{"index":0,"delta":{"content":"Voy a "}}]}),
                    json!({"choices":[{"index":0,"delta":{"content":"mirarlo."}}]}),
                    json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"get_current_time","arguments":"{\"tz\":"}}]}}]}),
                    json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"UTC\"}"}}]}}]}),
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
                ]),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let p = provider(&server.uri(), Some("clave-secreta"));
        let mut deltas = Vec::new();
        let request = ChatRequest {
            messages: vec![ChatMessage::system("sé breve"), ChatMessage::user("¿qué hora es?")],
            tools: vec![LlmTool { name: "get_current_time".into(), description: "hora".into(), parameters: json!({"type":"object"}) }],
        };
        let r = p.chat_stream(request, &mut |d| deltas.push(d), &CancellationToken::new()).await.expect("chat");
        assert_eq!(r.content, "Voy a mirarlo.");
        assert_eq!(r.tool_calls, vec![ToolCall { id: "call_abc".into(), name: "get_current_time".into(), arguments: json!({"tz":"UTC"}) }]);
        assert_eq!(deltas.len(), 2);
    }

    #[test]
    fn tool_results_reference_the_call_they_answer() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = vec![
            ToolCall { id: "c1".into(), name: "a".into(), arguments: json!({}) },
            ToolCall { id: "c2".into(), name: "b".into(), arguments: json!({"x":1}) },
        ];
        let encoded = encode_messages(&[
            ChatMessage::system("s1"),
            ChatMessage::user("hola"),
            assistant,
            ChatMessage::tool("b", "resultado b"),
            ChatMessage::tool("a", "resultado a"),
        ]);
        assert_eq!(encoded[0]["role"], "system");
        assert_eq!(encoded[2]["tool_calls"][1]["function"]["arguments"], "{\"x\":1}");
        assert_eq!(encoded[3]["tool_call_id"], "c2");
        assert_eq!(encoded[4]["tool_call_id"], "c1");
    }

    #[tokio::test]
    async fn errors_are_typed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":{"message":"invalid api key"}})))
            .mount(&server)
            .await;
        let p = provider(&server.uri(), None);
        let e = p
            .chat_stream(ChatRequest { messages: vec![ChatMessage::user("x")], tools: vec![] }, &mut |_| {}, &CancellationToken::new())
            .await
            .expect_err("401");
        assert_eq!(e, LlmError::Http { status: 401, body: "invalid api key".into() });
        let down = provider("http://127.0.0.1:9", None);
        assert!(matches!(down.health().await, Err(LlmError::Unavailable(_))));
    }
}
