//! Jarvis desktop: agent core local-first.
//!
//! ```text
//! React (UI) ──invoke/Channel──► commands ──► AgentOrchestrator
//!                                              ├─ LlmPlanner ──► OllamaProvider (Qwen)
//!                                              ├─ PolicyEngine + ApprovalGate
//!                                              ├─ ToolRegistry ──► herramientas locales
//!                                              │                └► RemoteTool ──pipe──► windows-agent
//!                                              └─ MemoryEngine (SQLite: grafo, embeddings, habilidades)
//! MemoryEngine ──broadcast──► app.emit("jarvis://visual") ──► Neural Core (React Three Fiber)
//! ```

pub mod commands;
pub mod config;
pub mod core;
pub mod domain;
pub mod infrastructure;
pub mod memory;
pub mod sync;
pub mod state;
pub mod tools;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::config::AppConfig;
use crate::state::AppState;

/// Punto de entrada de la aplicación.
///
/// # Panics
/// Si Tauri no puede iniciar la ventana.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "jarvis_desktop_lib=info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let config = AppConfig::load().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            tracing::info!(data_dir = %config.data_dir.display(), config_dir = ?config.config_dir, "configuración cargada");
            let state = Arc::new(AppState::build(config).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?);
            app.manage(state.clone());

            // Eventos visuales del Memory Engine y del agente → toda la UI.
            let mut events = state.memory.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            if let Err(e) = handle.emit("jarvis://visual", &event) {
                                tracing::debug!(error = %e, "no se pudo emitir el evento visual");
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(n, "eventos visuales descartados por lentitud");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Modelo de embeddings: se comprueba en segundo plano.
            let s2 = state.clone();
            tauri::async_runtime::spawn(async move {
                let detail = s2.select_embedder().await;
                tracing::info!(%detail, "embeddings");
            });
            // Sincronización con el Cloud Core: local primero; el bucle no hace nada en modo local o sin emparejar.
            let sync = state.sync.clone();
            let sync_cancel = tokio_util::sync::CancellationToken::new();
            tauri::async_runtime::spawn(sync.run_loop(sync_cancel));
            // Un cambio local despierta el bucle (los eventos de memoria se emiten tras cada escritura).
            let mut changes = state.memory.subscribe();
            let nudge = state.sync.clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = changes.recv().await {
                    if matches!(
                        event,
                        crate::domain::VisualEvent::MemoryCreated { .. }
                            | crate::domain::VisualEvent::MemoryUpdated { .. }
                            | crate::domain::VisualEvent::MemoryDeleted { .. }
                            | crate::domain::VisualEvent::MemoryConnected { .. }
                            | crate::domain::VisualEvent::SkillLearned { .. }
                    ) {
                        nudge.nudge();
                    }
                }
            });
            // El windows-agent es opcional: se conecta en segundo plano.
            tauri::async_runtime::spawn(async move {
                let status = state.refresh_windows_agent().await;
                tracing::info!(ok = status.ok, detail = %status.detail, "windows-agent");
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::chat::send_message,
            commands::chat::cancel_run,
            commands::chat::resolve_approval,
            commands::chat::list_conversations,
            commands::chat::get_messages,
            commands::chat::delete_conversation,
            commands::system::get_status,
            commands::system::list_models,
            commands::system::list_audit,
            commands::system::list_runs,
            commands::tools::list_tools,
            commands::tools::set_tool_mode,
            commands::tools::refresh_windows_agent,
            commands::memory::list_memories,
            commands::memory::get_memory,
            commands::memory::add_memory,
            commands::memory::update_memory,
            commands::memory::delete_memory,
            commands::memory::set_memory_verification,
            commands::memory::add_relation,
            commands::memory::verify_relation,
            commands::memory::delete_relation,
            commands::memory::memory_graph,
            commands::memory::search_memories,
            commands::memory::memory_stats,
            commands::memory::consolidate_memory,
            commands::memory::refresh_embeddings,
            commands::memory::cancel_memory_job,
            commands::memory::list_experiences,
            commands::memory::promote_experience,
            commands::memory::list_skills,
            commands::memory::skill_versions,
            commands::memory::set_skill_enabled,
            commands::memory::update_skill,
            commands::memory::default_export_path,
            commands::memory::list_memory_packages,
            commands::memory::export_memory,
            commands::memory::preview_import,
            commands::memory::apply_import,
            commands::memory::list_backups,
            commands::memory::create_backup,
            commands::memory::restore_backup,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::voice::voice_connection,
            commands::cloud::cloud_status,
            commands::cloud::cloud_probe,
            commands::cloud::cloud_pair,
            commands::cloud::cloud_unpair,
            commands::cloud::cloud_service_credential,
            commands::cloud::cloud_default_url,
            commands::cloud::cloud_set_mode,
            commands::cloud::cloud_sync_now,
            commands::cloud::cloud_conflicts,
            commands::cloud::cloud_resolve_conflict,
            commands::cloud::get_inference,
            commands::cloud::update_inference,
            commands::cloud::test_inference,
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar Jarvis");
}
