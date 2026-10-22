use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use super::{err, CmdResult};
use crate::core::policies::{Decision, RunSecurity, ToolMode};
use crate::domain::{RiskClass, ToolSource};
use crate::state::{AgentLinkStatus, AppState};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub risk: RiskClass,
    pub permissions: Vec<String>,
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub source: ToolSource,
    pub parameters: Value,
    pub policy_mode: ToolMode,
    pub effective_decision: &'static str,
}

#[tauri::command]
pub fn list_tools(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<ToolInfo>> {
    let policy = state.policy.read().map_err(err)?.clone();
    Ok(state
        .registry
        .specs()
        .into_iter()
        .map(|s| {
            let decision = policy.decide(&s, &RunSecurity::default());
            ToolInfo {
                policy_mode: policy.mode(&s.name),
                effective_decision: match decision {
                    Decision::Allow => "allow",
                    Decision::Confirm { .. } => "confirm",
                    Decision::Deny { .. } => "deny",
                },
                id: s.id,
                name: s.name,
                description: s.description,
                risk: s.risk,
                permissions: s.permissions,
                timeout_ms: s.timeout_ms,
                cancellable: s.cancellable,
                source: s.source,
                parameters: s.parameters,
            }
        })
        .collect())
}

#[tauri::command]
pub fn set_tool_mode(state: State<'_, Arc<AppState>>, tool: String, mode: ToolMode) -> CmdResult<()> {
    {
        let mut cfg = state.config.lock().map_err(err)?;
        if mode == ToolMode::Default {
            cfg.user.tool_modes.remove(&tool);
        } else {
            cfg.user.tool_modes.insert(tool.clone(), mode);
        }
        cfg.persist()?;
    }
    state.policy.write().map_err(err)?.set_tool_mode(&tool, mode);
    state.db.audit(&crate::infrastructure::persistence::audit::AuditRecord {
        actor: "user".into(),
        action: "set_tool_mode".into(),
        tool: Some(tool),
        decision: Some(format!("{mode:?}").to_lowercase()),
        ..Default::default()
    }).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn refresh_windows_agent(state: State<'_, Arc<AppState>>) -> CmdResult<AgentLinkStatus> {
    Ok(state.refresh_windows_agent().await)
}
