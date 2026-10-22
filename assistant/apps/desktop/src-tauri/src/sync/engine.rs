//! Motor de sincronización del escritorio (ADR-015): local primero, push del outbox y pull incremental cuando hay
//! conexión, reintentos con backoff y conflictos visibles. Jarvis funciona igual sin Cloud Core.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::client::{backoff, CloudClient, CloudError, DeviceSecretSource, Timeouts};
use super::protocol::{
    PulledChange, PushRequest, QueryEmbedding, ResolveRequest, SearchRequest, SearchResponse, ALL_SCOPES, MAX_CHANGES_PER_PUSH,
    MAX_PUSH_BYTES, PROTOCOL,
};
use super::secrets::SecretVault;
use super::store::{Applied, LocalConflict, OutboxStats, SyncStore};
use crate::memory::MemoryEngine;

pub const SECRET_KEY: &str = "cloud-device-secret";
const PULL_PAGE: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    /// Sin Cloud Core: todo queda en esta PC.
    Local,
    /// Local primero y sincronización con el Cloud Core cuando hay conexión.
    Hybrid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub mode: SyncMode,
    pub paired: bool,
    pub base_url: Option<String>,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub vault: String,
    pub online: bool,
    pub latency_ms: Option<u64>,
    pub vector_search: Option<String>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub pending: u64,
    pub rejected: u64,
    pub conflicts: usize,
    pub cursor: i64,
    pub syncing: bool,
    pub local_memories: u64,
    pub synced_memories: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub pushed: usize,
    pub merged: usize,
    pub conflicts: usize,
    pub rejected: usize,
    pub pulled: usize,
    pub memories_changed: usize,
    pub memories_deleted: usize,
}

/// Credencial del device: id en `sync_meta`, secreto en el almacén del sistema.
struct VaultSecrets {
    store: SyncStore,
    vault: Arc<dyn SecretVault>,
}

impl DeviceSecretSource for VaultSecrets {
    fn device_id(&self) -> Option<String> {
        self.store.meta("device_id").ok().flatten()
    }

    fn secret(&self) -> Option<String> {
        self.vault.get(SECRET_KEY)
    }
}

pub struct SyncEngine {
    store: SyncStore,
    secrets: VaultSecrets,
    memory: Arc<MemoryEngine>,
    client: RwLock<Option<Arc<CloudClient>>>,
    status: RwLock<SyncStatus>,
    run_lock: Mutex<()>,
    wake: Notify,
    timeouts: Timeouts,
}

impl SyncEngine {
    #[must_use]
    pub fn new(memory: Arc<MemoryEngine>, vault: Arc<dyn SecretVault>) -> Arc<Self> {
        Self::with_timeouts(memory, vault, Timeouts::default())
    }

    #[must_use]
    pub fn with_timeouts(memory: Arc<MemoryEngine>, vault: Arc<dyn SecretVault>, timeouts: Timeouts) -> Arc<Self> {
        let store = SyncStore::new(memory.database().clone());
        let engine = Arc::new(Self {
            secrets: VaultSecrets { store: store.clone(), vault: vault.clone() },
            store,
            memory,
            client: RwLock::new(None),
            status: RwLock::new(SyncStatus {
                mode: SyncMode::Local,
                paired: false,
                base_url: None,
                device_id: None,
                device_name: None,
                vault: vault.kind().into(),
                online: false,
                latency_ms: None,
                vector_search: None,
                last_sync_at: None,
                last_error: None,
                pending: 0,
                rejected: 0,
                conflicts: 0,
                cursor: 0,
                syncing: false,
                local_memories: 0,
                synced_memories: 0,
            }),
            run_lock: Mutex::new(()),
            wake: Notify::new(),
            timeouts,
        });
        if let Some(url) = engine.store.meta("base_url").ok().flatten() {
            if let Ok(client) = CloudClient::new(&url, timeouts) {
                *engine.client.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(client));
            }
        }
        engine.refresh_local_status();
        engine
    }

    fn client(&self) -> Option<Arc<CloudClient>> {
        self.client.read().ok().and_then(|c| c.clone())
    }

    fn mode(&self) -> SyncMode {
        match self.store.meta("mode").ok().flatten().as_deref() {
            Some("hybrid") => SyncMode::Hybrid,
            _ => SyncMode::Local,
        }
    }

    fn update(&self, f: impl FnOnce(&mut SyncStatus)) {
        if let Ok(mut s) = self.status.write() {
            f(&mut s);
        }
    }

    fn refresh_local_status(&self) {
        let stats = self.store.outbox_stats().unwrap_or_default();
        let conflicts = self.store.open_conflicts().map(|c| c.len()).unwrap_or(0);
        let paired = self.secrets.device_id().is_some() && self.secrets.secret().is_some();
        let mode = self.mode();
        let base_url = self.store.meta("base_url").ok().flatten();
        let device_id = self.secrets.device_id();
        let device_name = self.store.meta("device_name").ok().flatten();
        let cursor = self.store.cursor().unwrap_or(0);
        let (local_memories, synced_memories) = self.store.memory_counts().unwrap_or((0, 0));
        let last_sync = self.store.meta("last_sync_at").ok().flatten().and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.with_timezone(&Utc));
        self.update(|s| {
            s.mode = mode;
            s.paired = paired;
            s.base_url = base_url;
            s.device_id = device_id;
            s.device_name = device_name;
            s.pending = stats.pending;
            s.rejected = stats.rejected;
            s.conflicts = conflicts;
            s.cursor = cursor;
            s.local_memories = local_memories;
            s.synced_memories = synced_memories;
            s.last_sync_at = last_sync.or(s.last_sync_at);
            if s.last_error.is_none() {
                s.last_error = stats.last_error.clone();
            }
        });
    }

    #[must_use]
    pub fn status(&self) -> SyncStatus {
        self.refresh_local_status();
        self.status.read().map(|s| s.clone()).unwrap_or_else(|p| p.into_inner().clone())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn outbox(&self) -> Result<OutboxStats, String> {
        self.store.outbox_stats()
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn conflicts(&self) -> Result<Vec<LocalConflict>, String> {
        self.store.open_conflicts()
    }

    /// Comprueba el Cloud Core (latencia y capacidades) sin autenticarse.
    ///
    /// # Errors
    /// URL inválida o Cloud no disponible.
    pub async fn probe(&self, base_url: Option<&str>) -> Result<(u64, String), CloudError> {
        let client = match base_url {
            Some(url) => Arc::new(CloudClient::new(url, self.timeouts)?),
            None => self.client().ok_or_else(|| CloudError::Config("Cloud Core sin configurar".into()))?,
        };
        let (health, latency) = client.health().await?;
        let ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);
        self.update(|s| {
            s.online = true;
            s.latency_ms = Some(ms);
            s.vector_search = Some(health.features.vector_search.clone());
        });
        Ok((ms, health.features.vector_search))
    }

    /// Empareja este equipo: inicia sesión como owner (la contraseña no se guarda), se registra como device, guarda el
    /// secreto en el almacén del sistema, activa la captura de cambios y encola todo lo existente.
    ///
    /// # Errors
    /// Cloud no disponible, credenciales inválidas, nombre en uso…
    pub async fn pair(&self, base_url: &str, email: &str, password: &str, device_name: &str) -> Result<SyncStatus, CloudError> {
        let client = Arc::new(CloudClient::new(base_url, self.timeouts)?);
        client.health().await?;
        let registered = client.pair(email, password, device_name, &ALL_SCOPES).await?;
        self.secrets.vault.set(SECRET_KEY, &registered.credential.secret).map_err(CloudError::Config)?;
        let store = &self.store;
        let save = || -> Result<(), String> {
            store.set_meta("base_url", client.base_url())?;
            store.set_meta("device_id", &registered.device.id)?;
            store.set_meta("device_name", &registered.device.name)?;
            store.set_meta("mode", "hybrid")?;
            store.set_meta("cursor", "0")?;
            store.enable_and_enqueue_all().map(|_| ())
        };
        save().map_err(CloudError::Config)?;
        *self.client.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(client);
        self.update(|s| s.last_error = None);
        self.wake.notify_one();
        Ok(self.status())
    }

    /// Olvida el emparejamiento en esta PC (no borra recuerdos). Para impedir que el secreto se use, revoca el
    /// device desde el Cloud Core.
    ///
    /// # Errors
    /// Errores del almacén o de SQLite.
    pub fn unpair(&self) -> Result<SyncStatus, String> {
        self.secrets.vault.delete(SECRET_KEY)?;
        self.store.disable_and_forget()?;
        *self.client.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.update(|s| {
            s.online = false;
            s.latency_ms = None;
            s.last_error = None;
            s.last_sync_at = None;
        });
        Ok(self.status())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn set_mode(&self, mode: SyncMode) -> Result<SyncStatus, String> {
        self.store.set_meta("mode", if mode == SyncMode::Hybrid { "hybrid" } else { "local" })?;
        if mode == SyncMode::Hybrid {
            self.wake.notify_one();
        }
        Ok(self.status())
    }

    /// Despierta el bucle (tras un cambio local o a pedido del usuario).
    pub fn nudge(&self) {
        self.wake.notify_one();
    }

    /// Un ciclo completo: push del outbox y pull hasta el final.
    ///
    /// # Errors
    /// Errores de red (reintentables) o del Cloud.
    pub async fn run_once(&self) -> Result<SyncReport, CloudError> {
        let _guard = self.run_lock.lock().await;
        if self.mode() != SyncMode::Hybrid {
            return Ok(SyncReport::default());
        }
        let client = self.client().ok_or_else(|| CloudError::Config("Cloud Core sin configurar".into()))?;
        if self.secrets.device_id().is_none() || self.secrets.secret().is_none() {
            return Err(CloudError::Config("este equipo no está emparejado".into()));
        }
        self.update(|s| s.syncing = true);
        let result = self.cycle(&client).await;
        let now = Utc::now();
        match &result {
            Ok(_) => {
                let _ = self.store.set_meta("last_sync_at", &now.to_rfc3339());
                self.update(|s| {
                    s.online = true;
                    s.last_sync_at = Some(now);
                    s.last_error = None;
                    s.syncing = false;
                });
            }
            Err(e) => {
                let message = e.to_string();
                self.update(|s| {
                    s.online = !matches!(e, CloudError::Unreachable(_));
                    s.last_error = Some(message);
                    s.syncing = false;
                });
            }
        }
        self.refresh_local_status();
        result
    }

    async fn cycle(&self, client: &CloudClient) -> Result<SyncReport, CloudError> {
        let mut report = SyncReport::default();
        let mut merged = Applied::default();

        // Push: lotes de ≤ 200 cambios y ≤ ~900 KiB, hasta vaciar el outbox.
        loop {
            let batch = self.store.next_batch(MAX_CHANGES_PER_PUSH, MAX_PUSH_BYTES).map_err(CloudError::Config)?;
            if batch.changes.is_empty() {
                break;
            }
            let request = PushRequest { protocol: PROTOCOL.into(), changes: batch.changes.clone() };
            match client.push(&self.secrets, &request).await {
                Ok(response) => {
                    for r in &response.results {
                        match r.status.as_str() {
                            "applied" => report.pushed += 1,
                            "merged" => report.merged += 1,
                            "conflict" => report.conflicts += 1,
                            _ => report.rejected += 1,
                        }
                    }
                    let applied = self.store.acknowledge(&batch, &response.results).map_err(CloudError::Config)?;
                    merged.upserted_memories.extend(applied.upserted_memories);
                }
                Err(e) => {
                    let _ = self.store.record_attempt_error(&batch, &e.to_string());
                    return Err(e);
                }
            }
        }

        // Pull: hasta que no haya más; las relaciones con extremos aún ausentes se reintentan al final.
        let model = self.memory.embedder();
        let local_model = model.is_semantic().then(|| model.model_id());
        let mut applied = Applied::default();
        let mut deferred: Vec<PulledChange> = Vec::new();
        let mut since = self.store.cursor().map_err(CloudError::Config)?;
        loop {
            let page = client.pull(&self.secrets, since, PULL_PAGE, local_model.iter().cloned().collect()).await?;
            report.pulled += page.changes.len();
            deferred.extend(self.store.apply_pulled(&page.changes, local_model.as_deref(), &mut applied).map_err(CloudError::Config)?);
            since = page.next_since;
            self.store.set_meta("cursor", &since.to_string()).map_err(CloudError::Config)?;
            if !page.has_more {
                break;
            }
        }
        if !deferred.is_empty() {
            let still = self.store.apply_pulled(&deferred, local_model.as_deref(), &mut applied).map_err(CloudError::Config)?;
            if !still.is_empty() {
                tracing::warn!(n = still.len(), "relaciones recibidas sin sus recuerdos; se omiten");
            }
        }

        let mut upserted = merged.upserted_memories;
        upserted.extend(applied.upserted_memories.iter().cloned());
        upserted.sort();
        upserted.dedup();
        report.memories_changed = upserted.len();
        report.memories_deleted = applied.deleted_memories.len();
        if !upserted.is_empty() || !applied.deleted_memories.is_empty() || applied.relations > 0 {
            self.memory.notify_remote(&upserted, &applied.deleted_memories);
        }
        if !upserted.is_empty() {
            // Los recuerdos recibidos se vectorizan con el modelo local (los vectores del mismo modelo ya llegaron).
            if let Err(e) = self.memory.reembed(&CancellationToken::new()).await {
                tracing::warn!(error = %e, "vectores de recuerdos recibidos pendientes; se calcularán después");
            }
        }
        Ok(report)
    }

    /// Bucle en segundo plano: cada 30 s o al recibir un aviso; con backoff ante errores de red; se detiene ante una
    /// credencial rechazada hasta que el usuario actúe.
    pub async fn run_loop(self: Arc<Self>, cancel: CancellationToken) {
        let mut failures = 0u32;
        loop {
            let wait = match self.run_once().await {
                Ok(_) => {
                    failures = 0;
                    Duration::from_secs(30)
                }
                Err(e) if e.is_retryable() => {
                    failures += 1;
                    backoff(failures)
                }
                Err(CloudError::Config(_)) => Duration::from_secs(60),
                Err(e) => {
                    tracing::warn!(error = %e, "sincronización detenida hasta una acción del usuario");
                    Duration::from_secs(3600)
                }
            };
            tokio::select! {
                () = cancel.cancelled() => return,
                () = self.wake.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// Recuperación en el Cloud durante una conversación (modo híbrido). Nunca bloquea: timeout corto y `None`
    /// si no hay conexión.
    pub async fn remote_search(&self, query: &str, embedding: Option<(String, Vec<f32>)>, limit: u32) -> Option<SearchResponse> {
        if self.mode() != SyncMode::Hybrid {
            return None;
        }
        let client = self.client()?;
        let request = SearchRequest {
            query: query.chars().take(1000).collect(),
            embedding: embedding.map(|(model, vector)| QueryEmbedding { model, vector }),
            limit: Some(limit),
            min_confidence: None,
            expand_graph: Some(true),
        };
        match client.search(&self.secrets, &request, Duration::from_millis(1500)).await {
            Ok(response) => Some(response),
            Err(e) => {
                tracing::debug!(error = %e, "búsqueda en el Cloud Core omitida");
                None
            }
        }
    }

    /// Resuelve un conflicto en el Cloud y trae la versión resultante.
    ///
    /// # Errors
    /// Cloud no disponible o resolución inválida.
    pub async fn resolve_conflict(&self, conflict_id: &str, resolution: &str, data: Option<serde_json::Value>) -> Result<SyncStatus, CloudError> {
        let client = self.client().ok_or_else(|| CloudError::Config("Cloud Core sin configurar".into()))?;
        let result = client.resolve(&self.secrets, conflict_id, &ResolveRequest { resolution: resolution.into(), data }).await;
        match result {
            Ok(_) => self.store.mark_conflict(conflict_id, "resolved").map_err(CloudError::Config)?,
            Err(CloudError::Rejected { status: 409, .. }) => self.store.mark_conflict(conflict_id, "resolved").map_err(CloudError::Config)?,
            Err(e) => return Err(e),
        }
        self.run_once().await?;
        Ok(self.status())
    }
}
