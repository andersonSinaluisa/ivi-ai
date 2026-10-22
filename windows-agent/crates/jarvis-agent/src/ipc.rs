//! Canal local de la CLI: named pipe con ACL del usuario, JSON por línea (docs/01-architecture/component-model.md#cli-local).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::health::Snapshot;
use crate::pipeline::Agent;

pub const MAX_COMMAND_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Status,
    Stop,
    ApprovalsList,
    ApprovalsDecide { task_id: Uuid, granted: bool },
    Reconnect,
    ClearSafeMode,
    PolicyReload,
    AckStateReset,
}

// Etiquetado adyacente: una variante puede contener una lista (las etiquetas internas solo
// admiten mapas, y fallar al serializar dejaría al cliente sin respuesta).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum Response {
    Status(Box<Snapshot>),
    Approvals(Vec<ApprovalView>),
    Ok { message: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalView {
    pub task_id: Uuid,
    pub tool: String,
    pub task_type: String,
    pub principal: String,
    pub risk_level: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Resumen del input sin campos sensibles (solo claves conocidas y cortas).
    pub input_summary: String,
}

/// Acciones que la CLI puede pedir al proceso residente.
pub struct IpcHandler {
    pub agent: Arc<Agent>,
    pub policy: Arc<crate::policy_watch::PolicyReloader>,
    pub shutdown: CancellationToken,
    pub reconnect: Arc<tokio::sync::Notify>,
    pub version: &'static str,
}

impl IpcHandler {
    pub async fn handle(&self, command: Command) -> Response {
        match command {
            Command::Status => {
                let stats = self.agent.store.stats().unwrap_or_default();
                let pending = self.agent.store.pending_approvals().map(|a| a.len()).unwrap_or(0);
                let db = crate::health::DbSection {
                    healthy: true,
                    size_bytes: stats.size_bytes,
                    outbox_pending: stats.outbox_pending,
                    unsettled: stats.unsettled,
                };
                let tools = crate::health::ToolsSection {
                    healthy: self.agent.executor.stuck() == 0,
                    enabled: self.agent.registry.enabled_count(),
                    running: self.agent.executor.running(),
                    awaiting_approval: pending,
                    stuck: self.agent.executor.stuck(),
                };
                Response::Status(Box::new(self.agent.health.snapshot(db, tools, self.version)))
            }
            Command::Stop => {
                self.shutdown.cancel();
                Response::Ok { message: "stopping".into() }
            }
            Command::Reconnect => {
                // `notify_one` deja permiso guardado: `notify_waiters` se pierde si el enlace no
                // esta justo esperando, y la reconexion pedida no llegaria a ocurrir.
                self.reconnect.notify_one();
                Response::Ok { message: "reconnecting".into() }
            }
            Command::AckStateReset => {
                self.agent.health.clear(crate::health::codes::STATE_RESET);
                Response::Ok { message: "state reset acknowledged".into() }
            }
            Command::PolicyReload => match self.policy.reload() {
                Ok(message) => Response::Ok { message },
                Err(error) => {
                    Response::Error { message: format!("policy.toml is invalid, the previous one is kept: {error}") }
                }
            },
            Command::ClearSafeMode => match crate::safe_mode::clear(&self.agent.store) {
                Ok(()) => {
                    self.agent.set_safe_mode(false);
                    self.agent.health.clear(crate::health::codes::SAFE_MODE);
                    self.agent.health.set_agent_state(crate::health::AgentState::Running);
                    Response::Ok { message: "safe mode cleared".into() }
                }
                Err(error) => Response::Error { message: error.to_string() },
            },
            Command::ApprovalsList => match self.agent.store.pending_approvals() {
                Ok(pending) => Response::Approvals(
                    pending
                        .into_iter()
                        .map(|item| ApprovalView {
                            task_id: item.approval.task_id,
                            tool: item.tool.unwrap_or_else(|| "-".into()),
                            task_type: item.task_type,
                            principal: item.principal,
                            risk_level: item.risk_level.unwrap_or_else(|| "-".into()),
                            expires_at: item.approval.expires_at,
                            input_summary: summarize(&item.payload),
                        })
                        .collect(),
                ),
                Err(error) => Response::Error { message: error.to_string() },
            },
            Command::ApprovalsDecide { task_id, granted } => {
                match self.agent.decide_approval(task_id, granted, "local:owner").await {
                    Ok(()) => Response::Ok {
                        message: format!("task {task_id} {}", if granted { "approved" } else { "denied" }),
                    },
                    Err(error) => Response::Error { message: error.to_string() },
                }
            }
        }
    }
}

/// Resumen corto del payload para que el owner sepa qué aprueba (sin volcar datos sensibles).
fn summarize(payload: &serde_json::Value) -> String {
    let Some(object) = payload.as_object() else { return String::new() };
    object
        .iter()
        .filter(|(key, _)| matches!(key.as_str(), "appId" | "force" | "ifRunning" | "commandId"))
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bucle del servidor: una conexión a la vez, un comando por conexión.
#[cfg(windows)]
pub async fn serve(handler: Arc<IpcHandler>, pipe_name: String, user_sid: String, cancel: CancellationToken) {
    let mut first = true;
    while !cancel.is_cancelled() {
        let server = match jarvis_windows::pipe::create_server(&pipe_name, &user_sid, first) {
            Ok(server) => server,
            Err(error) => {
                tracing::error!(%error, "cannot create the local pipe");
                return;
            }
        };
        first = false;
        tokio::select! {
            () = cancel.cancelled() => return,
            result = server.connect() => {
                if let Err(error) = result {
                    tracing::warn!(%error, "pipe connection failed");
                    continue;
                }
            }
        }
        let handler = handler.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(server);
            let mut line = String::new();
            let read = tokio::time::timeout(std::time::Duration::from_secs(10), reader.read_line(&mut line)).await;
            let response = match read {
                Ok(Ok(bytes)) if bytes > 0 && bytes <= MAX_COMMAND_BYTES => match serde_json::from_str(line.trim()) {
                    Ok(command) => handler.handle(command).await,
                    Err(error) => Response::Error { message: format!("invalid command: {error}") },
                },
                _ => Response::Error { message: "no command received".into() },
            };
            let mut payload = serde_json::to_vec(&response).unwrap_or_else(|error| {
                tracing::error!(%error, "cannot serialize the response");
                serde_json::to_vec(&Response::Error { message: "internal error".into() })
                    .unwrap_or_else(|_| b"{}".to_vec())
            });
            payload.push(b'\n');
            let mut server = reader.into_inner();
            let _ = server.write_all(&payload).await;
            let _ = server.flush().await;
        });
    }
}

/// Cliente de la CLI.
///
/// # Errors
/// Si el agente no está corriendo o el pipe no responde.
#[cfg(windows)]
pub async fn send(pipe_name: &str, command: &Command) -> anyhow::Result<Response> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let client = ClientOptions::new().open(pipe_name).map_err(|e| anyhow::anyhow!("the agent is not running ({e})"))?;
    let mut payload = serde_json::to_vec(command)?;
    payload.push(b'\n');
    let mut reader = BufReader::new(client);
    reader.get_mut().write_all(&payload).await?;
    reader.get_mut().flush().await?;
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(15), reader.read_line(&mut line)).await??;
    Ok(serde_json::from_str(line.trim())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_responses_round_trip() {
        let command = Command::ApprovalsDecide { task_id: Uuid::nil(), granted: true };
        let text = serde_json::to_string(&command).expect("serialize");
        assert!(text.contains("approvals_decide"));
        assert_eq!(serde_json::from_str::<Command>(&text).expect("parse"), command);
    }

    /// Toda respuesta debe poder serializarse: un fallo dejaría al cliente sin datos.
    #[test]
    fn every_response_variant_round_trips() {
        let approval = ApprovalView {
            task_id: Uuid::nil(),
            tool: "close_app".into(),
            task_type: "CLOSE_APP".into(),
            principal: "cloud:user:x".into(),
            risk_level: "R3".into(),
            expires_at: chrono::Utc::now(),
            input_summary: "appId=notepad force=true".into(),
        };
        let responses = [
            Response::Approvals(vec![approval]),
            Response::Approvals(Vec::new()),
            Response::Ok { message: "done".into() },
            Response::Error { message: "nope".into() },
        ];
        for response in responses {
            let text = serde_json::to_string(&response).expect("serializable");
            assert_eq!(serde_json::from_str::<Response>(&text).expect("parse"), response);
        }
    }

    #[test]
    fn approval_summaries_only_expose_known_fields() {
        let payload = serde_json::json!({"appId": "spotify", "force": true, "secretNote": "CANARY"});
        let summary = summarize(&payload);
        assert!(summary.contains("appId"));
        assert!(summary.contains("force"));
        assert!(!summary.contains("CANARY"));
    }
}
