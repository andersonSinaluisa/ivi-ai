//! Local Tool API (EPIC-16): `tools/list` y `tools/call` por named pipe, en JSON-RPC 2.0.
//!
//! Es el punto de entrada del futuro `jarvis-agent-core` (ADR-014). Aquí **no** hay razonamiento ni
//! nada de IA: es otro frontend del mismo pipeline, con la misma política, los mismos locks, la
//! misma verificación y la misma auditoría que una tarea del Cloud.
//!
//! Dos decisiones de seguridad propias de este canal:
//!
//! - el principal es siempre `local:agent-core`, nunca `local:owner`: quien se conecta es un
//!   proceso del usuario, y un LLM manipulable por *prompt injection* no debe poder atribuirse los
//!   permisos del owner (el owner actúa por su propio canal, la CLI);
//! - hay un presupuesto de llamadas por minuto, para que un bucle del modelo no se lleve la PC por
//!   delante.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jarvis_policy::Principal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use crate::pipeline::Agent;

/// Tamaño máximo de una petición (el input de una tool está acotado por su propio esquema).
pub const MAX_REQUEST_BYTES: usize = 128 * 1024;
/// Versión del contrato que anuncia `tools/list`.
pub const API_VERSION: &str = "jarvis.local-tool-api/1";

// Códigos de error de JSON-RPC 2.0 más los propios del dominio.
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
/// La política local no permite esa llamada.
pub const NOT_PERMITTED: i32 = -32000;
/// Presupuesto de llamadas agotado.
pub const RATE_LIMITED: i32 = -32001;
/// La tool se ejecutó pero falló (el detalle va en `data`).
pub const TOOL_FAILED: i32 = -32002;

#[derive(Debug, Clone, Deserialize)]
struct Request {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Serialize)]
struct Success {
    jsonrpc: &'static str,
    id: Value,
    result: Value,
}

#[derive(Debug, Clone, Serialize)]
struct Failure {
    jsonrpc: &'static str,
    id: Value,
    error: ErrorObject,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorObject {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

fn success(id: Value, result: Value) -> String {
    serde_json::to_string(&Success { jsonrpc: "2.0", id, result })
        .unwrap_or_else(|_| String::from(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal"}}"#))
}

fn failure(id: Value, code: i32, message: impl Into<String>, data: Option<Value>) -> String {
    let error = ErrorObject { code, message: message.into(), data };
    serde_json::to_string(&Failure { jsonrpc: "2.0", id, error })
        .unwrap_or_else(|_| String::from(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal"}}"#))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
    /// Permite reintentar sin repetir el efecto: la misma id devuelve el resultado anterior.
    #[serde(default)]
    invocation_id: Option<String>,
}

/// Presupuesto de llamadas por minuto (ventana deslizante).
#[derive(Debug)]
struct Budget {
    limit: usize,
    calls: Mutex<VecDeque<Instant>>,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self { limit, calls: Mutex::new(VecDeque::new()) }
    }

    /// `true` si la llamada cabe en el presupuesto.
    fn admit(&self) -> bool {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        while calls.front().is_some_and(|first| now.duration_since(*first) > Duration::from_secs(60)) {
            calls.pop_front();
        }
        if calls.len() >= self.limit {
            return false;
        }
        calls.push_back(now);
        true
    }
}

/// Resultados ya devueltos, para que un reintento con la misma `invocationId` no repita el efecto.
///
/// Es memoria del proceso: si el agente se reinicia, el Agent Core tendrá que consultar el estado
/// por sus propios medios. Se documenta así en [Agent Core Integration](../../docs/08-future/agent-core-integration.md).
#[derive(Debug)]
struct Recent {
    entries: Mutex<VecDeque<(String, Value)>>,
    capacity: usize,
}

impl Recent {
    fn new(capacity: usize) -> Self {
        Self { entries: Mutex::new(VecDeque::new()), capacity }
    }

    fn get(&self, key: &str) -> Option<Value> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.iter().find(|(id, _)| id == key).map(|(_, value)| value.clone())
    }

    fn put(&self, key: String, value: Value) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() == self.capacity {
            entries.pop_front();
        }
        entries.push_back((key, value));
    }
}

/// Estado del servidor de la API local.
pub struct ToolApi {
    agent: Arc<Agent>,
    budget: Budget,
    recent: Recent,
}

impl ToolApi {
    #[must_use]
    pub fn new(agent: Arc<Agent>, calls_per_minute: usize) -> Arc<Self> {
        Arc::new(Self { agent, budget: Budget::new(calls_per_minute.max(1)), recent: Recent::new(64) })
    }

    /// Procesa una línea JSON-RPC y devuelve la respuesta, también en una línea.
    pub async fn handle_line(&self, line: &str) -> String {
        let request: Request = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => return failure(Value::Null, PARSE_ERROR, format!("invalid JSON: {error}"), None),
        };
        if request.jsonrpc != "2.0" {
            return failure(request.id, INVALID_REQUEST, "only JSON-RPC 2.0 is supported", None);
        }

        match request.method.as_str() {
            "tools/list" => success(request.id, self.list()),
            "tools/call" => self.call(request.id, request.params).await,
            other => failure(request.id, METHOD_NOT_FOUND, format!("unknown method '{other}'"), None),
        }
    }

    /// Descriptores de las tools habilitadas, en la forma que espera un cliente tipo MCP.
    fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .agent
            .registry
            .descriptors()
            .into_iter()
            .filter(|(_, enabled)| *enabled)
            .map(|(descriptor, _)| {
                json!({
                    "name": descriptor.name,
                    "version": descriptor.version,
                    "description": descriptor.description,
                    "inputSchema": descriptor.input_schema,
                    "outputSchema": descriptor.output_schema,
                    "riskLevel": descriptor.risk_level.to_string(),
                    "requiredPermissions": descriptor.required_permissions,
                    "requiresInteractiveSession": descriptor.requires_interactive_session,
                    "sensitivity": descriptor.sensitivity,
                })
            })
            .collect();
        json!({ "apiVersion": API_VERSION, "principal": Principal::LocalAgentCore.policy_id(), "tools": tools })
    }

    async fn call(&self, id: Value, params: Value) -> String {
        let params: CallParams = match serde_json::from_value(params) {
            Ok(params) => params,
            Err(error) => return failure(id, INVALID_PARAMS, format!("invalid params: {error}"), None),
        };

        // Reintento de una llamada ya atendida: se devuelve el resultado anterior sin repetir nada.
        if let Some(key) = &params.invocation_id
            && let Some(previous) = self.recent.get(key)
        {
            return success(id, json!({ "result": previous, "deduplicated": true }));
        }

        if !self.budget.admit() {
            return failure(id, RATE_LIMITED, "too many tool calls: slow down", None);
        }

        match self.agent.invoke_local(&params.name, params.arguments, &Principal::LocalAgentCore).await {
            Ok(result) => {
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                if let Some(key) = params.invocation_id {
                    self.recent.put(key, value.clone());
                }
                // Un `ToolResult` que no tuvo éxito viaja como error JSON-RPC con el resultado
                // completo en `data`: el llamante ve por qué falló, con sus checks.
                if result.outcome == jarvis_tooling::Outcome::Succeeded {
                    success(id, json!({ "result": value }))
                } else {
                    failure(id, TOOL_FAILED, "the tool did not succeed", Some(value))
                }
            }
            Err(error) => {
                let message = error.to_string();
                let code = if message.contains("DENIED") || message.contains("REQUIRES_APPROVAL") {
                    NOT_PERMITTED
                } else {
                    INVALID_PARAMS
                };
                failure(id, code, message, None)
            }
        }
    }
}

/// Nombre del pipe de la API local para ese usuario.
#[must_use]
pub fn pipe_name(user_sid: &str) -> String {
    format!(r"\\.\pipe\jarvis-tools-{user_sid}")
}

/// Sirve la API local hasta que se cancele. Una conexión a la vez, varias peticiones por conexión.
#[cfg(windows)]
pub async fn serve(api: Arc<ToolApi>, pipe: String, user_sid: String, cancel: CancellationToken) {
    let mut first = true;
    while !cancel.is_cancelled() {
        let server = match jarvis_windows::pipe::create_server(&pipe, &user_sid, first) {
            Ok(server) => server,
            Err(error) => {
                tracing::error!(%error, "cannot create the local tool API pipe");
                return;
            }
        };
        first = false;
        tokio::select! {
            () = cancel.cancelled() => return,
            result = server.connect() => {
                if let Err(error) = result {
                    tracing::warn!(%error, "local tool API connection failed");
                    continue;
                }
            }
        }

        let api = api.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(server);
            let mut line = String::new();
            loop {
                line.clear();
                let read = tokio::select! {
                    () = cancel.cancelled() => return,
                    read = reader.read_line(&mut line) => read,
                };
                match read {
                    Ok(0) => return,
                    Ok(bytes) if bytes <= MAX_REQUEST_BYTES => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let mut response = api.handle_line(line.trim()).await.into_bytes();
                        response.push(b'\n');
                        let writer = reader.get_mut();
                        if writer.write_all(&response).await.is_err() || writer.flush().await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {
                        let mut response =
                            failure(Value::Null, INVALID_REQUEST, "request too large", None).into_bytes();
                        response.push(b'\n');
                        let _ = reader.get_mut().write_all(&response).await;
                        return;
                    }
                    Err(_) => return,
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_is_a_sliding_window() {
        let budget = Budget::new(3);
        assert!(budget.admit() && budget.admit() && budget.admit(), "las tres primeras entran");
        assert!(!budget.admit(), "la cuarta no");
    }

    #[test]
    fn recent_results_are_bounded_and_returned_by_id() {
        let recent = Recent::new(2);
        recent.put("a".into(), json!(1));
        recent.put("b".into(), json!(2));
        assert_eq!(recent.get("a"), Some(json!(1)));

        recent.put("c".into(), json!(3));
        assert_eq!(recent.get("a"), None, "la más antigua se descarta");
        assert_eq!(recent.get("c"), Some(json!(3)));
    }

    #[test]
    fn error_responses_follow_json_rpc() {
        let text = failure(json!(7), METHOD_NOT_FOUND, "unknown method 'x'", None);
        let parsed: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(parsed["jsonrpc"], json!("2.0"));
        assert_eq!(parsed["id"], json!(7));
        assert_eq!(parsed["error"]["code"], json!(METHOD_NOT_FOUND));
        assert!(parsed.get("result").is_none(), "un error no lleva result");
    }

    #[test]
    fn the_pipe_name_is_per_user() {
        assert_eq!(pipe_name("S-1-5-21-1"), r"\\.\pipe\jarvis-tools-S-1-5-21-1");
        assert_ne!(pipe_name("S-1-5-21-1"), pipe_name("S-1-5-21-2"));
    }
}
