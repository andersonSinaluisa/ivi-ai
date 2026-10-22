//! Cliente de la Local Tool API del windows-agent.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::Mutex;

/// Errores JSON-RPC propios del windows-agent.
pub const NOT_PERMITTED: i64 = -32000;
pub const RATE_LIMITED: i64 = -32001;
pub const TOOL_FAILED: i64 = -32002;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ToolApiError {
    #[error("windows-agent no disponible: {0}")]
    Unavailable(String),
    #[error("la política del windows-agent no lo permite: {0}")]
    NotPermitted(String),
    #[error("demasiadas llamadas por minuto al windows-agent")]
    RateLimited,
    #[error("la herramienta falló: {message}")]
    ToolFailed { message: String, data: Option<Value> },
    #[error("error JSON-RPC {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("respuesta inválida: {0}")]
    Protocol(String),
}

/// Transporte de líneas JSON: una petición, una respuesta.
#[async_trait]
pub trait LineTransport: Send {
    async fn round_trip(&mut self, line: &str) -> Result<String, ToolApiError>;
}

/// Crea conexiones nuevas (para reconectar tras un fallo).
#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn LineTransport>, ToolApiError>;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub version: Option<Value>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub risk_level: String,
    #[serde(default)]
    pub required_permissions: Vec<String>,
    #[serde(default)]
    pub sensitivity: Option<Value>,
}

pub struct ToolApiClient {
    connector: Arc<dyn Connector>,
    conn: Mutex<Option<Box<dyn LineTransport>>>,
    next_id: std::sync::atomic::AtomicU64,
    timeout: Duration,
}

impl ToolApiClient {
    #[must_use]
    pub fn new(connector: Arc<dyn Connector>, timeout: Duration) -> Self {
        Self { connector, conn: Mutex::new(None), next_id: std::sync::atomic::AtomicU64::new(1), timeout }
    }

    /// Envía una petición. Si la conexión se rompió, reconecta **una** vez y reintenta.
    async fn request(&self, method: &str, params: Value) -> Result<Value, ToolApiError> {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();

        let mut guard = self.conn.lock().await;
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(self.connector.connect().await?);
            }
            let Some(conn) = guard.as_mut() else { continue };
            match tokio::time::timeout(self.timeout, conn.round_trip(&line)).await {
                Ok(Ok(response)) => return parse_response(&response),
                Ok(Err(ToolApiError::Unavailable(e))) if attempt == 0 => {
                    tracing::warn!(error = %e, "conexión con el windows-agent perdida; reconectando");
                    *guard = None;
                }
                Ok(Err(e)) => {
                    *guard = None;
                    return Err(e);
                }
                Err(_) => {
                    *guard = None;
                    return Err(ToolApiError::Unavailable("tiempo de espera agotado".into()));
                }
            }
        }
        Err(ToolApiError::Unavailable("no se pudo reconectar".into()))
    }

    /// # Errors
    /// Si el agente no está disponible o responde algo inesperado.
    pub async fn list_tools(&self) -> Result<Vec<RemoteToolDescriptor>, ToolApiError> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result.get("tools").cloned().ok_or_else(|| ToolApiError::Protocol("falta 'tools'".into()))?;
        serde_json::from_value(tools).map_err(|e| ToolApiError::Protocol(e.to_string()))
    }

    /// Ejecuta una herramienta. `invocation_id` permite reintentar sin repetir el efecto.
    ///
    /// # Errors
    /// Ver [`ToolApiError`].
    pub async fn call(&self, name: &str, arguments: Value, invocation_id: &str) -> Result<Value, ToolApiError> {
        let result = self
            .request("tools/call", json!({"name": name, "arguments": arguments, "invocationId": invocation_id}))
            .await?;
        result.get("result").cloned().ok_or_else(|| ToolApiError::Protocol("falta 'result'".into()))
    }
}

fn parse_response(text: &str) -> Result<Value, ToolApiError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ToolApiError::Protocol(e.to_string()))?;
    if let Some(error) = v.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error.get("message").and_then(Value::as_str).unwrap_or("").to_owned();
        return Err(match code {
            NOT_PERMITTED => ToolApiError::NotPermitted(message),
            RATE_LIMITED => ToolApiError::RateLimited,
            TOOL_FAILED => ToolApiError::ToolFailed { message, data: error.get("data").cloned() },
            _ => ToolApiError::Rpc { code, message },
        });
    }
    v.get("result").cloned().ok_or_else(|| ToolApiError::Protocol("respuesta sin 'result' ni 'error'".into()))
}

/// Nombre del pipe para el usuario actual.
///
/// # Errors
/// Si no se puede obtener el SID.
pub async fn default_pipe_name() -> Result<String, ToolApiError> {
    let sid = current_user_sid().await?;
    Ok(format!(r"\\.\pipe\jarvis-tools-{sid}"))
}

/// SID del usuario con `whoami /user /fo csv /nh` → `"equipo\usuario","S-1-5-21-…"`.
async fn current_user_sid() -> Result<String, ToolApiError> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("whoami");
        cmd.args(["/user", "/fo", "csv", "/nh"]);
        // Sin ventana de consola.
        cmd.creation_flags(0x0800_0000);
        let out = cmd.output().await.map_err(|e| ToolApiError::Unavailable(format!("whoami: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        parse_sid(&text).ok_or_else(|| ToolApiError::Unavailable("no se pudo leer el SID del usuario".into()))
    }
    #[cfg(not(windows))]
    {
        Err(ToolApiError::Unavailable("la Local Tool API solo existe en Windows".into()))
    }
}

#[must_use]
pub fn parse_sid(whoami_csv: &str) -> Option<String> {
    whoami_csv
        .trim()
        .split(',')
        .nth(1)
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| s.starts_with("S-1-"))
}

/// Conector por named pipe (solo Windows).
pub struct NamedPipeConnector {
    pub pipe: String,
}

#[cfg(windows)]
mod pipe_impl {
    use super::{async_trait, Connector, LineTransport, NamedPipeConnector, ToolApiError};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    struct PipeTransport {
        reader: BufReader<NamedPipeClient>,
    }

    #[async_trait]
    impl LineTransport for PipeTransport {
        async fn round_trip(&mut self, line: &str) -> Result<String, ToolApiError> {
            let unavailable = |e: std::io::Error| ToolApiError::Unavailable(e.to_string());
            let pipe = self.reader.get_mut();
            pipe.write_all(line.as_bytes()).await.map_err(unavailable)?;
            pipe.write_all(b"\n").await.map_err(unavailable)?;
            pipe.flush().await.map_err(unavailable)?;
            let mut response = String::new();
            let n = self.reader.read_line(&mut response).await.map_err(unavailable)?;
            if n == 0 {
                return Err(ToolApiError::Unavailable("el windows-agent cerró la conexión".into()));
            }
            Ok(response)
        }
    }

    #[async_trait]
    impl Connector for NamedPipeConnector {
        async fn connect(&self) -> Result<Box<dyn LineTransport>, ToolApiError> {
            let client = ClientOptions::new().open(&self.pipe).map_err(|e| {
                ToolApiError::Unavailable(format!(
                    "no se pudo abrir {} ({e}). ¿Está corriendo jarvis-agent con [tool_api] enabled = true?",
                    self.pipe
                ))
            })?;
            Ok(Box::new(PipeTransport { reader: BufReader::new(client) }))
        }
    }
}

#[cfg(not(windows))]
#[async_trait]
impl Connector for NamedPipeConnector {
    async fn connect(&self) -> Result<Box<dyn LineTransport>, ToolApiError> {
        Err(ToolApiError::Unavailable(format!("{}: los named pipes de Windows no existen en este sistema", self.pipe)))
    }
}

#[cfg(test)]
pub mod testing {
    //! Windows-agent falso en memoria para pruebas.
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub type Handler = Arc<dyn Fn(&Value) -> Value + Send + Sync>;

    pub struct FakeConnector {
        pub handler: Handler,
        pub connects: AtomicUsize,
        /// Cuántas peticiones aguanta cada conexión antes de "romperse".
        pub break_after: Option<usize>,
    }

    struct FakeTransport {
        handler: Handler,
        remaining: Option<usize>,
    }

    #[async_trait]
    impl LineTransport for FakeTransport {
        async fn round_trip(&mut self, line: &str) -> Result<String, ToolApiError> {
            if let Some(r) = self.remaining.as_mut() {
                if *r == 0 {
                    return Err(ToolApiError::Unavailable("pipe roto".into()));
                }
                *r -= 1;
            }
            let req: Value = serde_json::from_str(line).map_err(|e| ToolApiError::Protocol(e.to_string()))?;
            let mut resp = (self.handler)(&req);
            resp["jsonrpc"] = json!("2.0");
            resp["id"] = req["id"].clone();
            Ok(resp.to_string())
        }
    }

    #[async_trait]
    impl Connector for FakeConnector {
        async fn connect(&self) -> Result<Box<dyn LineTransport>, ToolApiError> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeTransport { handler: self.handler.clone(), remaining: self.break_after }))
        }
    }

    /// Un windows-agent con `list_processes` (R0) y `close_app` (R3).
    #[must_use]
    pub fn fake_agent(break_after: Option<usize>) -> Arc<FakeConnector> {
        Arc::new(FakeConnector {
            connects: AtomicUsize::new(0),
            break_after,
            handler: Arc::new(|req: &Value| match req["method"].as_str() {
                Some("tools/list") => json!({"result": {"apiVersion": "jarvis.local-tool-api/1", "principal": "local:agent-core", "tools": [
                    {"name": "list_processes", "version": 1, "description": "Lista procesos", "inputSchema": {"type": "object"}, "riskLevel": "R0", "requiredPermissions": ["process.read"]},
                    {"name": "close_app", "version": 1, "description": "Cierra una app", "inputSchema": {"type": "object"}, "riskLevel": "R3", "requiredPermissions": ["process.control"]}
                ]}}),
                Some("tools/call") => match req["params"]["name"].as_str() {
                    Some("list_processes") => json!({"result": {"result": {"schema": "jarvis.tool-result/1", "tool": "list_processes", "outcome": "SUCCEEDED", "output": {"processes": ["notepad.exe"]}, "verification": {"status": "PASSED", "method": "none", "durationMs": 0, "checks": []}}}}),
                    Some("close_app") => json!({"error": {"code": NOT_PERMITTED, "message": "REQUIRES_APPROVAL"}}),
                    _ => json!({"error": {"code": -32602, "message": "unknown tool"}}),
                },
                _ => json!({"error": {"code": -32601, "message": "unknown method"}}),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fake_agent;
    use super::*;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn lists_and_calls_tools() {
        let client = ToolApiClient::new(fake_agent(None), Duration::from_secs(1));
        let tools = client.list_tools().await.expect("list");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[1].risk_level, "R3");
        let r = client.call("list_processes", json!({}), "inv-1").await.expect("call");
        assert_eq!(r["outcome"], "SUCCEEDED");
        let e = client.call("close_app", json!({"appId": "notepad"}), "inv-2").await.expect_err("denied");
        assert_eq!(e, ToolApiError::NotPermitted("REQUIRES_APPROVAL".into()));
    }

    #[tokio::test]
    async fn reconnects_once_when_the_pipe_breaks() {
        let connector = fake_agent(Some(1));
        let client = ToolApiClient::new(connector.clone(), Duration::from_secs(1));
        client.list_tools().await.expect("first");
        // La conexión ya no acepta más: el cliente debe reconectar y reintentar.
        client.list_tools().await.expect("second after reconnect");
        assert_eq!(connector.connects.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn parses_the_sid_from_whoami() {
        assert_eq!(
            parse_sid("\"pc\\anderson\",\"S-1-5-21-111-222-333-1001\"\r\n").as_deref(),
            Some("S-1-5-21-111-222-333-1001")
        );
        assert_eq!(parse_sid("basura"), None);
    }
}
