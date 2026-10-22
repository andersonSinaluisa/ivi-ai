//! Cloud Core (sincronización) e Inference Gateway: Configuración → Infraestructura, Memoria y Modelos.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{err, CmdResult};
use crate::config::InferenceSettings;
use crate::infrastructure::llm::gateway::GatewayStatus;
use crate::infrastructure::llm::LlmProvider;
use crate::state::{configure_gateway, AppState, INFERENCE_KEY};
use crate::sync::store::LocalConflict;
use crate::sync::{SyncMode, SyncReport, SyncStatus};

#[tauri::command]
pub fn cloud_status(state: State<'_, Arc<AppState>>) -> CmdResult<SyncStatus> {
    Ok(state.sync.status())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub latency_ms: u64,
    pub vector_search: String,
}

/// Comprueba un Cloud Core (sin credenciales): latencia y capacidades.
#[tauri::command]
pub async fn cloud_probe(state: State<'_, Arc<AppState>>, url: Option<String>) -> CmdResult<ProbeResult> {
    let (latency_ms, vector_search) = state.sync.probe(url.as_deref()).await.map_err(err)?;
    Ok(ProbeResult { latency_ms, vector_search })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub url: String,
    pub email: String,
    pub password: String,
    pub device_name: String,
}

/// Empareja este equipo. La contraseña solo se usa para esta llamada; nunca se guarda.
#[tauri::command]
pub async fn cloud_pair(state: State<'_, Arc<AppState>>, request: PairRequest) -> CmdResult<SyncStatus> {
    let name = request.device_name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("el nombre del equipo debe tener entre 1 y 64 caracteres".into());
    }
    let status = state.sync.pair(&request.url, request.email.trim(), &request.password, name).await.map_err(err)?;
    // Modelos, embeddings y voz pasan a usar el servidor recién emparejado.
    state.apply_settings();
    let s = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let detail = s.select_embedder().await;
        tracing::info!(%detail, "memoria tras emparejar");
    });
    Ok(status)
}

#[tauri::command]
pub fn cloud_unpair(state: State<'_, Arc<AppState>>) -> CmdResult<SyncStatus> {
    let status = state.sync.unpair()?;
    state.apply_settings();
    Ok(status)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCredential {
    /// URL del servidor emparejado; la voz usa `<url>/stt` y `<url>/tts`.
    pub server_url: String,
    /// Token de acceso del device (caduca; la UI lo vuelve a pedir antes y lo entrega al servicio de voz local).
    pub token: String,
}

/// Credencial para que el servicio de voz local (127.0.0.1) hable con el servidor en nombre de este equipo.
#[tauri::command]
pub async fn cloud_service_credential(state: State<'_, Arc<AppState>>) -> CmdResult<ServiceCredential> {
    let server_url = state.sync.server_url().ok_or("este equipo no está emparejado con tu servidor")?;
    let token = state.sync.service_token().await.map_err(err)?;
    Ok(ServiceCredential { server_url, token })
}

/// Servidor que se propone al emparejar (`[cloud] default_url` de jarvis.toml).
#[tauri::command]
pub fn cloud_default_url(state: State<'_, Arc<AppState>>) -> CmdResult<String> {
    Ok(state.config.lock().map_err(err)?.file.cloud.default_url.clone())
}

#[tauri::command]
pub fn cloud_set_mode(state: State<'_, Arc<AppState>>, mode: SyncMode) -> CmdResult<SyncStatus> {
    state.sync.set_mode(mode)
}

#[tauri::command]
pub async fn cloud_sync_now(state: State<'_, Arc<AppState>>) -> CmdResult<SyncReport> {
    state.sync.run_once().await.map_err(err)
}

#[tauri::command]
pub fn cloud_conflicts(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<LocalConflict>> {
    state.sync.conflicts()
}

#[tauri::command]
pub async fn cloud_resolve_conflict(
    state: State<'_, Arc<AppState>>,
    id: String,
    resolution: String,
    data: Option<serde_json::Value>,
) -> CmdResult<SyncStatus> {
    if !matches!(resolution.as_str(), "keep_server" | "keep_device" | "custom") {
        return Err("resolución inválida".into());
    }
    state.sync.resolve_conflict(&id, &resolution, data).await.map_err(err)
}

// ---------------------------------------------------------------------- Inference Gateway

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceView {
    pub settings: InferenceSettings,
    pub has_api_key: bool,
    pub status: GatewayStatus,
    pub vault: String,
}

fn view(state: &AppState) -> CmdResult<InferenceView> {
    let settings = state.config.lock().map_err(err)?.user.inference.clone();
    Ok(InferenceView {
        settings,
        has_api_key: state.vault.get(INFERENCE_KEY).is_some(),
        status: state.gateway.status(),
        vault: state.vault.kind().into(),
    })
}

#[tauri::command]
pub fn get_inference(state: State<'_, Arc<AppState>>) -> CmdResult<InferenceView> {
    view(&state)
}

/// Guarda el proveedor de inferencia. `api_key`: `Some("")` la borra, `None` la deja como está.
#[tauri::command]
pub fn update_inference(state: State<'_, Arc<AppState>>, settings: InferenceSettings, api_key: Option<String>) -> CmdResult<InferenceView> {
    settings.validate()?;
    match api_key.as_deref().map(str::trim) {
        Some("") => state.vault.delete(INFERENCE_KEY)?,
        Some(key) => state.vault.set(INFERENCE_KEY, key)?,
        None => {}
    }
    {
        let mut cfg = state.config.lock().map_err(err)?;
        cfg.user.inference = settings;
        cfg.persist()?;
        configure_gateway(&state.gateway, &cfg, state.vault.as_ref());
    }
    view(&state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceTest {
    pub ok: bool,
    pub detail: String,
}

/// Prueba la cadena de proveedores del modo actual (sin enviar ningún mensaje).
#[tauri::command]
pub async fn test_inference(state: State<'_, Arc<AppState>>) -> CmdResult<InferenceTest> {
    Ok(match state.gateway.health().await {
        Ok(h) => InferenceTest { ok: true, detail: h.version },
        Err(e) => InferenceTest { ok: false, detail: e.to_string() },
    })
}
