//! Datos de conexión con el servicio de voz local.
//!
//! El servicio de voz genera un token aleatorio en `%LOCALAPPDATA%\Jarvis\voice-token` (solo
//! legible por el usuario). La UI lo necesita para autenticarse en el WebSocket.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::{err, CmdResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConnection {
    pub base_url: String,
    pub token: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn voice_connection(state: State<'_, Arc<AppState>>) -> CmdResult<VoiceConnection> {
    let cfg = state.config.lock().map_err(err)?;
    let base_url = cfg.file.voice.base_url.clone();
    let path = cfg.data_dir.join("voice-token");
    Ok(match std::fs::read_to_string(&path) {
        Ok(t) if !t.trim().is_empty() => VoiceConnection { base_url, token: Some(t.trim().to_owned()), error: None },
        _ => VoiceConnection {
            base_url,
            token: None,
            error: Some("El servicio de voz no está corriendo (no hay token). Inícialo con scripts\\start-voice.ps1.".into()),
        },
    })
}
