//! Comandos de memoria: explorar, editar, relacionar, grafo, experiencias, habilidades,
//! exportar, importar, respaldos y mantenimiento.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tauri::State;
use tokio_util::sync::CancellationToken;

use super::{err, CmdResult};
use crate::domain::{
    Experience, GraphView, MemoryKind, MemoryPatch, MemoryRecord, MemoryRelation, NewMemory, Provenance, RelationKind,
    RetentionPolicy, Skill, SkillVersion, Verification,
};
use crate::memory::memory_consolidator::ConsolidationReport;
use crate::memory::memory_engine::MemoryStats;
use crate::memory::memory_exporter::{BackupInfo, ExportOptions, ExportReport};
use crate::memory::memory_importer::{ImportPreview, ImportReport, ImportSelection};
use crate::memory::memory_package::KdfParams;
use crate::memory::memory_repository::{MemoryFilter, MemoryPage};
use crate::memory::memory_retriever::{RetrievalOptions, RetrievalResult};
use crate::memory::semantic_memory::{MemoryDetail, RememberOutcome};
use crate::state::AppState;

#[tauri::command]
pub fn list_memories(state: State<'_, Arc<AppState>>, filter: Option<MemoryFilter>, offset: Option<u64>, limit: Option<u64>) -> CmdResult<MemoryPage> {
    state.memory.list_memories(&filter.unwrap_or_default(), offset.unwrap_or(0), limit.unwrap_or(50)).map_err(err)
}

#[tauri::command]
pub fn get_memory(state: State<'_, Arc<AppState>>, id: String) -> CmdResult<MemoryDetail> {
    state.memory.memory_detail(&id).map_err(err)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMemoryInput {
    pub content: String,
    pub summary: Option<String>,
    pub kind: Option<MemoryKind>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub retention: Option<RetentionPolicy>,
}

#[tauri::command]
pub async fn add_memory(state: State<'_, Arc<AppState>>, input: AddMemoryInput) -> CmdResult<RememberOutcome> {
    let mut m = NewMemory::user(input.kind.unwrap_or(MemoryKind::Semantic), input.content);
    m.summary = input.summary;
    m.tags = input.tags;
    if let Some(r) = input.retention {
        m.retention = r;
    }
    state.memory.remember(m).await.map_err(err)
}

#[tauri::command]
pub async fn update_memory(state: State<'_, Arc<AppState>>, id: String, patch: MemoryPatch) -> CmdResult<MemoryRecord> {
    state.memory.update_memory(&id, patch).await.map_err(err)
}

#[tauri::command]
pub fn delete_memory(state: State<'_, Arc<AppState>>, id: String) -> CmdResult<()> {
    state.memory.delete_memory(&id).map_err(err)
}

#[tauri::command]
pub fn add_relation(state: State<'_, Arc<AppState>>, source_id: String, target_id: String, kind: RelationKind, note: Option<String>) -> CmdResult<MemoryRelation> {
    state.memory.relate(&source_id, &target_id, kind, Provenance::User, note).map_err(err)
}

#[tauri::command]
pub fn verify_relation(state: State<'_, Arc<AppState>>, id: String, verified: bool) -> CmdResult<MemoryRelation> {
    state.memory.set_relation_verified(&id, verified).map_err(err)
}

#[tauri::command]
pub fn delete_relation(state: State<'_, Arc<AppState>>, id: String) -> CmdResult<()> {
    state.memory.delete_relation(&id).map_err(err)
}

/// Vista del grafo: general (paginada) o centrada en un recuerdo.
#[tauri::command]
pub fn memory_graph(state: State<'_, Arc<AppState>>, focus: Option<String>, depth: Option<usize>, limit: Option<usize>, offset: Option<usize>) -> CmdResult<GraphView> {
    let limit = limit.unwrap_or(400);
    match focus {
        Some(id) => state.memory.graph_focus(&id, depth.unwrap_or(2), limit),
        None => state.memory.graph_overview(limit, offset.unwrap_or(0)),
    }
    .map_err(err)
}

/// Búsqueda manual (no se registra como uso del agente).
#[tauri::command]
pub async fn search_memories(state: State<'_, Arc<AppState>>, query: String) -> CmdResult<RetrievalResult> {
    let opts = RetrievalOptions { record_usage: false, limit: 12, token_budget: 2_000, ..RetrievalOptions::default() };
    state.memory.retrieve(&query, &opts).await.map_err(err)
}

#[tauri::command]
pub fn memory_stats(state: State<'_, Arc<AppState>>) -> CmdResult<MemoryStats> {
    state.memory.stats().map_err(err)
}

#[tauri::command]
pub fn consolidate_memory(state: State<'_, Arc<AppState>>) -> CmdResult<ConsolidationReport> {
    state.memory.consolidate().map_err(err)
}

/// Vuelve a probar el modelo de embeddings y recalcula lo pendiente.
#[tauri::command]
pub async fn refresh_embeddings(state: State<'_, Arc<AppState>>) -> CmdResult<String> {
    Ok(state.select_embedder().await)
}

#[tauri::command]
pub fn cancel_memory_job(state: State<'_, Arc<AppState>>) -> CmdResult<bool> {
    Ok(state.memory_job.lock().map_err(err)?.as_ref().map(CancellationToken::cancel).is_some())
}

#[tauri::command]
pub fn list_experiences(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<Experience>> {
    state.memory.experiences(200).map_err(err)
}

#[tauri::command]
pub async fn promote_experience(state: State<'_, Arc<AppState>>, id: String) -> CmdResult<Skill> {
    state.memory.promote_experience(&id).await.map_err(err)
}

#[tauri::command]
pub fn list_skills(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<Skill>> {
    state.memory.skills().map_err(err)
}

#[tauri::command]
pub fn skill_versions(state: State<'_, Arc<AppState>>, id: String) -> CmdResult<Vec<SkillVersion>> {
    state.memory.skill_versions(&id).map_err(err)
}

#[tauri::command]
pub fn set_skill_enabled(state: State<'_, Arc<AppState>>, id: String, enabled: bool) -> CmdResult<Skill> {
    state.memory.set_skill_enabled(&id, enabled).map_err(err)
}

#[tauri::command]
pub fn update_skill(state: State<'_, Arc<AppState>>, id: String, procedure: String, notes: Option<String>) -> CmdResult<Skill> {
    state.memory.update_skill_procedure(&id, &procedure, notes.as_deref()).map_err(err)
}

// ------------------------------------------------------------------ exportar e importar

fn import_path(path: &str) -> CmdResult<PathBuf> {
    let p = PathBuf::from(path.trim().trim_matches('"'));
    if !p.is_absolute() {
        return Err("indica la ruta completa del archivo .jarvismem".into());
    }
    Ok(p)
}

#[tauri::command]
pub fn default_export_path(state: State<'_, Arc<AppState>>) -> CmdResult<String> {
    Ok(state.memory.default_export_path().display().to_string())
}

/// Paquetes `.jarvismem` en la carpeta de exportación.
#[tauri::command]
pub fn list_memory_packages(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<String>> {
    let dir = state.memory.default_export_path().parent().map(PathBuf::from).unwrap_or_default();
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jarvismem"))
        .map(|p| p.display().to_string())
        .collect();
    out.sort();
    out.reverse();
    Ok(out)
}

#[tauri::command]
pub async fn export_memory(state: State<'_, Arc<AppState>>, path: Option<String>, passphrase: String, options: Option<ExportOptions>) -> CmdResult<ExportReport> {
    let path = match path.filter(|p| !p.trim().is_empty()) {
        Some(p) => import_path(&p)?,
        None => state.memory.default_export_path(),
    };
    state.memory.export_memory(&path, &passphrase, &options.unwrap_or_default(), KdfParams::default()).await.map_err(err)
}

#[tauri::command]
pub async fn preview_import(state: State<'_, Arc<AppState>>, path: String, passphrase: String) -> CmdResult<ImportPreview> {
    let path = import_path(&path)?;
    let memory = state.memory.clone();
    // La derivación de clave y la lectura del paquete no deben bloquear el runtime.
    tauri::async_runtime::spawn_blocking(move || memory.preview_import(&path, &passphrase)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
pub async fn apply_import(state: State<'_, Arc<AppState>>, path: String, passphrase: String, selection: Option<ImportSelection>) -> CmdResult<ImportReport> {
    let path = import_path(&path)?;
    state.memory.apply_import(&path, &passphrase, &selection.unwrap_or_default()).await.map_err(err)
}

#[tauri::command]
pub fn list_backups(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<BackupInfo>> {
    state.memory.list_backups().map_err(err)
}

#[tauri::command]
pub fn create_backup(state: State<'_, Arc<AppState>>) -> CmdResult<String> {
    state.memory.backup("manual").map(|p| p.display().to_string()).map_err(err)
}

#[tauri::command]
pub fn restore_backup(state: State<'_, Arc<AppState>>, name: String) -> CmdResult<String> {
    state.memory.restore_backup(&name).map(|p| p.display().to_string()).map_err(err)
}

/// Marcar un recuerdo como verificado o dudoso sin tocar el resto.
#[tauri::command]
pub async fn set_memory_verification(state: State<'_, Arc<AppState>>, id: String, verification: Verification) -> CmdResult<MemoryRecord> {
    state.memory.update_memory(&id, MemoryPatch { verification: Some(verification), ..MemoryPatch::default() }).await.map_err(err)
}
