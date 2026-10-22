use std::sync::Arc;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;
use tokio_util::sync::CancellationToken;

use super::{err, CmdResult};
use crate::core::agent::{AgentRequest, FnSink};
use crate::domain::AgentEvent;
use crate::infrastructure::persistence::conversations::{Conversation, StoredMessage};
use crate::state::AppState;

/// Longitud máxima de una instrucción.
const MAX_INPUT: usize = 8_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub run_id: String,
    pub conversation_id: String,
}

/// Inicia una ejecución del agente. Devuelve enseguida; los eventos llegan por `on_event`.
#[tauri::command]
pub async fn send_message(
    state: State<'_, Arc<AppState>>,
    conversation_id: Option<String>,
    text: String,
    on_event: Channel<AgentEvent>,
) -> CmdResult<RunInfo> {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("el mensaje está vacío".into());
    }
    if text.chars().count() > MAX_INPUT {
        return Err(format!("el mensaje supera {MAX_INPUT} caracteres"));
    }
    let conversation_id = state.db.ensure_conversation(conversation_id.as_deref(), &text).map_err(err)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    state.runs.lock().map_err(err)?.insert(run_id.clone(), cancel.clone());

    let request = AgentRequest { run_id: run_id.clone(), conversation_id: conversation_id.clone(), text };
    let state_arc: Arc<AppState> = Arc::clone(&state);
    let sink = Arc::new(FnSink(move |e: AgentEvent| {
        if let Err(error) = on_event.send(e) {
            tracing::debug!(%error, "la UI ya no escucha esta ejecución");
        }
    }));
    tauri::async_runtime::spawn(async move {
        let id = request.run_id.clone();
        let _ = state_arc.orchestrator.run(request, sink, cancel).await;
        if let Ok(mut runs) = state_arc.runs.lock() {
            runs.remove(&id);
        }
    });
    Ok(RunInfo { run_id, conversation_id })
}

#[tauri::command]
pub fn cancel_run(state: State<'_, Arc<AppState>>, run_id: String) -> CmdResult<bool> {
    let token = state.runs.lock().map_err(err)?.get(&run_id).cloned();
    Ok(token.map(|t| t.cancel()).is_some())
}

#[tauri::command]
pub fn resolve_approval(state: State<'_, Arc<AppState>>, approval_id: String, approved: bool) -> CmdResult<bool> {
    Ok(state.orchestrator.resolve_approval(&approval_id, approved))
}

#[tauri::command]
pub fn list_conversations(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<Conversation>> {
    state.db.list_conversations(100).map_err(err)
}

#[tauri::command]
pub fn get_messages(state: State<'_, Arc<AppState>>, conversation_id: String) -> CmdResult<Vec<StoredMessage>> {
    state.db.recent_messages(&conversation_id, 500).map_err(err)
}

#[tauri::command]
pub fn delete_conversation(state: State<'_, Arc<AppState>>, conversation_id: String) -> CmdResult<bool> {
    state.db.delete_conversation(&conversation_id).map_err(err)
}
