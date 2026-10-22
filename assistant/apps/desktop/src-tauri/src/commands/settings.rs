use std::sync::Arc;

use tauri::State;

use super::{err, CmdResult};
use crate::config::Settings;
use crate::state::AppState;

#[tauri::command]
pub fn get_settings(state: State<'_, Arc<AppState>>) -> CmdResult<Settings> {
    Ok(state.config.lock().map_err(err)?.settings())
}

#[tauri::command]
pub fn update_settings(state: State<'_, Arc<AppState>>, settings: Settings) -> CmdResult<Settings> {
    let saved = {
        let mut cfg = state.config.lock().map_err(err)?;
        cfg.save_settings(&settings)?;
        cfg.settings()
    };
    state.apply_settings();
    Ok(saved)
}
