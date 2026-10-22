use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::{err, CmdResult};
use crate::domain::ToolSource;
use crate::infrastructure::llm::{LlmError, LlmProvider};
use crate::infrastructure::persistence::audit::AuditEntry;
use crate::infrastructure::persistence::runs::RunSummary;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaCheck {
    pub ok: bool,
    pub detail: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCheck {
    pub ok: bool,
    pub detail: String,
    pub name: String,
    pub installed: bool,
    pub alternatives: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsAgentCheck {
    pub ok: bool,
    pub detail: String,
    pub tool_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub ollama: OllamaCheck,
    pub model: ModelCheck,
    pub windows_agent: WindowsAgentCheck,
    pub database: Check,
    pub local_tools: usize,
}

/// Estado de las dependencias. Nunca falla: cada error se reporta en su sección para que la UI
/// lo muestre sin bloquear el resto.
#[tauri::command]
pub async fn get_status(state: State<'_, Arc<AppState>>) -> CmdResult<SystemStatus> {
    let model = state.provider.model();
    let (ollama, model_check) = match state.provider.health().await {
        Ok(h) => {
            let installed = state.provider.list_models().await.unwrap_or_default();
            let found = installed.iter().any(|m| m == &model || m == &format!("{model}:latest"));
            let alternatives: Vec<String> = installed.iter().filter(|m| m.starts_with("qwen")).cloned().collect();
            let detail = if found {
                "instalado".to_owned()
            } else if alternatives.is_empty() {
                format!("no está en el servidor. En el servidor: ollama pull {model}")
            } else {
                format!("no está en el servidor (ollama pull {model}) o elige uno de los Qwen que sí están")
            };
            (
                OllamaCheck { ok: true, detail: format!("Ollama {} en el servidor", h.version), version: Some(h.version) },
                ModelCheck { ok: found, detail, name: model, installed: found, alternatives },
            )
        }
        Err(e) => {
            let detail = match e {
                LlmError::Unavailable(m) => m,
                other => other.to_string(),
            };
            (
                OllamaCheck { ok: false, detail, version: None },
                ModelCheck { ok: false, detail: "sin comprobar (servidor no disponible)".into(), name: model, installed: false, alternatives: vec![] },
            )
        }
    };
    let link = state.agent_link.read().map(|l| l.clone()).unwrap_or_default();
    let database = match state.db.ping() {
        Ok(()) => Check { ok: true, detail: "SQLite OK".into() },
        Err(e) => Check { ok: false, detail: e.to_string() },
    };
    Ok(SystemStatus {
        ollama,
        model: model_check,
        windows_agent: WindowsAgentCheck { ok: link.ok, detail: link.detail, tool_count: link.tool_count },
        database,
        local_tools: state.registry.count(ToolSource::Local),
    })
}

#[tauri::command]
pub async fn list_models(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<String>> {
    state.provider.list_models().await.map_err(err)
}

#[tauri::command]
pub fn list_audit(state: State<'_, Arc<AppState>>, limit: Option<usize>) -> CmdResult<Vec<AuditEntry>> {
    state.db.list_audit(limit.unwrap_or(200).min(2000)).map_err(err)
}

#[tauri::command]
pub fn list_runs(state: State<'_, Arc<AppState>>, limit: Option<usize>) -> CmdResult<Vec<RunSummary>> {
    state.db.list_runs(limit.unwrap_or(100).min(1000)).map_err(err)
}
