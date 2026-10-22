//! Fachada del Memory Engine: estado compartido, embeddings, caché de vectores y eventos visuales.
//! Las operaciones están repartidas por archivo (`impl MemoryEngine` en cada módulo).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tokio::sync::broadcast;

use super::embedding_provider::{EmbeddingProvider, HashingEmbedder, OllamaEmbedder};
use super::memory_repository::MemoryRepository;
use super::{MemoryError, MemoryResult};
use crate::domain::VisualEvent;
use crate::infrastructure::persistence::Database;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Similitud mínima para crear una relación inferida RELATED_TO (embeddings semánticos).
    pub autolink_semantic: f32,
    /// Lo mismo con el respaldo léxico.
    pub autolink_lexical: f32,
    /// Máximo de relaciones inferidas por recuerdo nuevo.
    pub autolink_max: usize,
    /// Máximo de vectores en caché.
    pub max_cached_vectors: usize,
    pub backups_dir: PathBuf,
    pub exports_dir: PathBuf,
}

impl MemoryConfig {
    #[must_use]
    pub fn in_dir(data_dir: &std::path::Path) -> Self {
        Self {
            autolink_semantic: 0.6,
            autolink_lexical: 0.3,
            autolink_max: 3,
            max_cached_vectors: 50_000,
            backups_dir: data_dir.join("backups"),
            exports_dir: dirs::document_dir().unwrap_or_else(|| data_dir.to_path_buf()).join("Jarvis"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStatus {
    pub model: String,
    pub semantic: bool,
    pub detail: String,
    pub pending: u64,
}

#[derive(Default)]
struct VectorCache {
    model: String,
    loaded: bool,
    vectors: HashMap<String, Vec<f32>>,
}

pub struct MemoryEngine {
    pub(crate) repo: MemoryRepository,
    embedder: RwLock<Arc<dyn EmbeddingProvider>>,
    embed_detail: RwLock<String>,
    cache: RwLock<VectorCache>,
    events: broadcast::Sender<VisualEvent>,
    pub(crate) config: MemoryConfig,
    pub(crate) job_lock: tokio::sync::Mutex<()>,
}

impl MemoryEngine {
    /// # Errors
    /// Si falla la preparación del repositorio.
    pub fn new(db: Database, embedder: Arc<dyn EmbeddingProvider>, config: MemoryConfig) -> MemoryResult<Self> {
        let (events, _) = broadcast::channel(256);
        let detail = if embedder.is_semantic() {
            format!("semántico ({})", embedder.model_id())
        } else {
            "léxico: no entiende sinónimos. Instala un modelo de embeddings para búsqueda semántica".to_owned()
        };
        Ok(Self {
            repo: MemoryRepository::new(db)?,
            embedder: RwLock::new(embedder),
            embed_detail: RwLock::new(detail),
            cache: RwLock::new(VectorCache::default()),
            events,
            config,
            job_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Motor en memoria con respaldo léxico (pruebas).
    ///
    /// # Errors
    /// Errores de SQLite.
    #[cfg(test)]
    pub fn for_tests() -> MemoryResult<Self> {
        let db = Database::open_in_memory().map_err(|e| MemoryError::Storage(e.to_string()))?;
        let dir = std::env::temp_dir().join(format!("jarvis-test-{}", uuid::Uuid::new_v4()));
        Self::new(db, Arc::new(HashingEmbedder::default()), MemoryConfig::in_dir(&dir))
    }

    #[must_use]
    pub fn repository(&self) -> &MemoryRepository {
        &self.repo
    }

    #[must_use]
    pub fn database(&self) -> &Database {
        self.repo.db()
    }

    // ------------------------------------------------------------------ eventos

    pub fn emit(&self, event: VisualEvent) {
        // Sin suscriptores no es un error.
        let _ = self.events.send(event);
    }

    /// Cambios recibidos de otro device (sincronización): la vista los muestra como cualquier otro recuerdo nuevo,
    /// editado o borrado, y la caché de vectores se invalida.
    pub fn notify_remote(&self, upserted: &[String], deleted: &[String]) {
        self.invalidate_cache();
        for id in deleted {
            self.emit(VisualEvent::MemoryDeleted { memory_id: id.clone() });
        }
        let Ok(records) = self.repo.get_many(upserted) else { return };
        let degrees = self.repo.degrees(upserted).unwrap_or_default();
        for record in records {
            let edges = self.repo.relations_of(&record.id).unwrap_or_default();
            self.emit(VisualEvent::MemoryCreated {
                node: crate::domain::GraphNode::from_record(&record, degrees.get(&record.id).copied().unwrap_or(0)),
                edges: edges.iter().map(crate::domain::GraphEdge::from).collect(),
            });
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<VisualEvent> {
        self.events.subscribe()
    }

    // ------------------------------------------------------------------ embeddings

    #[must_use]
    pub fn embedder(&self) -> Arc<dyn EmbeddingProvider> {
        self.embedder.read().map(|e| e.clone()).unwrap_or_else(|p| p.into_inner().clone())
    }

    /// Cambia de proveedor. Los vectores del modelo anterior se conservan pero se ignoran.
    pub fn set_embedder(&self, embedder: Arc<dyn EmbeddingProvider>, detail: String) {
        if let Ok(mut e) = self.embedder.write() {
            *e = embedder;
        }
        if let Ok(mut d) = self.embed_detail.write() {
            *d = detail;
        }
        if let Ok(mut c) = self.cache.write() {
            *c = VectorCache::default();
        }
    }

    /// Prueba el modelo de embeddings de Ollama; si no responde, usa el respaldo léxico.
    pub async fn select_embedder(base_url: &str, model: &str, auth: crate::infrastructure::llm::Auth) -> (Arc<dyn EmbeddingProvider>, String) {
        match OllamaEmbedder::new(base_url, model).map(|e| e.with_auth(auth)) {
            Ok(e) => match e.embed(&["prueba".to_owned()]).await {
                Ok(_) => {
                    let id = e.model_id();
                    (Arc::new(e), format!("semántico ({id})"))
                }
                Err(err) => (
                    Arc::new(HashingEmbedder::default()),
                    format!("léxico (sin modelo semántico en el servidor: {err}). En el servidor: ollama pull {model}"),
                ),
            },
            Err(err) => (Arc::new(HashingEmbedder::default()), format!("léxico ({err})")),
        }
    }

    /// # Errors
    /// Errores de almacenamiento.
    pub fn embedding_status(&self) -> MemoryResult<EmbeddingStatus> {
        let e = self.embedder();
        Ok(EmbeddingStatus {
            model: e.model_id(),
            semantic: e.is_semantic(),
            detail: self.embed_detail.read().map(|d| d.clone()).unwrap_or_default(),
            pending: self.repo.count_missing_embeddings(&e.model_id())?,
        })
    }

    /// Texto que se vectoriza para un recuerdo.
    pub(crate) fn embedding_text(summary: &str, content: &str, tags: &[String]) -> String {
        let mut t = String::new();
        if !summary.trim().is_empty() && summary.trim() != content.trim() {
            t.push_str(summary.trim());
            t.push('\n');
        }
        t.push_str(content.trim());
        if !tags.is_empty() {
            t.push_str("\n#");
            t.push_str(&tags.join(" #"));
        }
        t
    }

    /// Calcula y guarda el vector de un recuerdo.
    ///
    /// # Errors
    /// [`MemoryError::Embedding`] si el proveedor falla.
    pub(crate) async fn embed_memory(&self, id: &str, hash: &str, text: &str) -> MemoryResult<Vec<f32>> {
        let embedder = self.embedder();
        let mut vectors =
            embedder.embed(&[text.to_owned()]).await.map_err(|e| MemoryError::Embedding(e.to_string()))?;
        let vector = vectors.pop().ok_or_else(|| MemoryError::Embedding("respuesta vacía".into()))?;
        self.repo.upsert_embedding(id, &embedder.model_id(), hash, &vector)?;
        self.cache_put(&embedder.model_id(), id, vector.clone());
        Ok(vector)
    }

    /// # Errors
    /// [`MemoryError::Embedding`] si el proveedor falla.
    pub(crate) async fn embed_query(&self, text: &str) -> MemoryResult<Vec<f32>> {
        let mut v = self.embedder().embed(&[text.to_owned()]).await.map_err(|e| MemoryError::Embedding(e.to_string()))?;
        v.pop().ok_or_else(|| MemoryError::Embedding("respuesta vacía".into()))
    }

    // ------------------------------------------------------------------ caché de vectores

    /// Vectores vigentes del modelo actual (se cargan de SQLite la primera vez).
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub(crate) fn vectors(&self) -> MemoryResult<Vec<(String, Vec<f32>)>> {
        let model = self.embedder().model_id();
        {
            let cache = self.cache.read().map_err(|_| MemoryError::Storage("caché envenenada".into()))?;
            if cache.loaded && cache.model == model {
                return Ok(cache.vectors.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
            }
        }
        let rows = self.repo.embeddings_for_model(&model)?;
        let mut cache = self.cache.write().map_err(|_| MemoryError::Storage("caché envenenada".into()))?;
        cache.model = model;
        cache.loaded = true;
        cache.vectors = rows.into_iter().take(self.config.max_cached_vectors).map(|(id, _, v)| (id, v)).collect();
        Ok(cache.vectors.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    fn cache_put(&self, model: &str, id: &str, vector: Vec<f32>) {
        if let Ok(mut c) = self.cache.write() {
            if c.loaded && c.model == model && c.vectors.len() < self.config.max_cached_vectors {
                c.vectors.insert(id.to_owned(), vector);
            }
        }
    }

    pub(crate) fn cache_remove(&self, id: &str) {
        if let Ok(mut c) = self.cache.write() {
            c.vectors.remove(id);
        }
    }

    pub(crate) fn invalidate_cache(&self) {
        if let Ok(mut c) = self.cache.write() {
            *c = VectorCache::default();
        }
    }

    /// Recalcula los embeddings que falten para el modelo actual (tras cambiar de modelo o importar).
    ///
    /// # Errors
    /// [`MemoryError::Cancelled`] si se cancela; errores del proveedor.
    pub async fn reembed(&self, cancel: &tokio_util::sync::CancellationToken) -> MemoryResult<usize> {
        let _guard = self.job_lock.lock().await;
        let model = self.embedder().model_id();
        let mut done = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(MemoryError::Cancelled);
            }
            let batch = self.repo.missing_embeddings(&model, 16)?;
            if batch.is_empty() {
                break;
            }
            let texts: Vec<String> = batch.iter().map(|m| Self::embedding_text(&m.summary, &m.content, &m.tags)).collect();
            let vectors = self.embedder().embed(&texts).await.map_err(|e| MemoryError::Embedding(e.to_string()))?;
            for (m, v) in batch.iter().zip(vectors) {
                self.repo.upsert_embedding(&m.id, &model, &m.content_hash, &v)?;
            }
            done += batch.len();
        }
        self.invalidate_cache();
        Ok(done)
    }

    /// Estadísticas para la pantalla de memoria.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub fn stats(&self) -> MemoryResult<MemoryStats> {
        let relations: i64 = self
            .database()
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_relations", [], |r| r.get(0))
            .map_err(super::memory_repository::storage)?;
        let skills: i64 = self
            .database()
            .conn()
            .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
            .map_err(super::memory_repository::storage)?;
        Ok(MemoryStats {
            total: self.repo.count()?,
            by_kind: self.repo.counts_by_kind()?,
            relations: u64::try_from(relations).unwrap_or(0),
            skills: u64::try_from(skills).unwrap_or(0),
            embedding: self.embedding_status()?,
            exports_dir: self.config.exports_dir.display().to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total: u64,
    pub by_kind: HashMap<String, u64>,
    pub relations: u64,
    pub skills: u64,
    pub embedding: EmbeddingStatus,
    pub exports_dir: String,
}
