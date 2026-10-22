//! Neural Memory Engine: memoria persistente, relacional y transferible.
//!
//! ```text
//! MemoryEngine ─┬─ memory_repository  (SQLite: recuerdos, fuentes, relaciones, embeddings, eventos)
//!               ├─ semantic_memory    (crear / editar / borrar recuerdos, sin secretos)
//!               ├─ episodic_memory    (historial de conversaciones)
//!               ├─ experience_memory  (tareas registradas, sin duplicados)
//!               ├─ procedural_memory  (habilidades versionadas; solo desde experiencias verificadas)
//!               ├─ memory_graph       (relaciones tipadas, vecinos, recorridos, subgrafos)
//!               ├─ memory_retriever   (embedding → similitud → grafo → filtros → ranking → contexto)
//!               ├─ memory_consolidator(retención, duplicados, relaciones débiles)
//!               ├─ embedding_provider (Ollama o respaldo léxico)
//!               └─ memory_exporter / memory_importer (paquete cifrado .jarvismem)
//! ```

pub mod embedding_provider;
pub mod episodic_memory;
pub mod experience_memory;
pub mod memory_consolidator;
pub mod memory_engine;
pub mod memory_exporter;
pub mod memory_graph;
pub mod memory_importer;
pub mod memory_package;
pub mod memory_repository;
pub mod memory_retriever;
pub mod procedural_memory;
pub mod secrets;
pub mod semantic_memory;
pub mod vector;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::{ChatMessage, Experience, Skill};

pub use memory_engine::{EmbeddingStatus, MemoryConfig, MemoryEngine};
pub use memory_retriever::{RetrievalOptions, RetrievalResult};

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("el contenido parece contener un secreto (contraseña, token o clave) y no se guarda")]
    ContainsSecret,
    #[error("recuerdo no encontrado")]
    NotFound,
    #[error("contenido inválido: {0}")]
    Invalid(String),
    #[error("la experiencia no está verificada o no terminó: no puede convertirse en habilidad")]
    NotVerified,
    #[error("embeddings: {0}")]
    Embedding(String),
    #[error("paquete de memoria: {0}")]
    Package(String),
    #[error("operación cancelada")]
    Cancelled,
    #[error("almacenamiento: {0}")]
    Storage(String),
}

pub type MemoryResult<T> = Result<T, MemoryError>;

/// Historial de conversaciones.
pub trait EpisodicMemory: Send + Sync {
    /// # Errors
    /// Errores de almacenamiento.
    fn record_message(&self, conversation_id: &str, message: &ChatMessage) -> MemoryResult<()>;
    /// # Errors
    /// Errores de almacenamiento.
    fn recent(&self, conversation_id: &str, limit: usize) -> MemoryResult<Vec<ChatMessage>>;
}

/// Recuperación de contexto para el agente.
#[async_trait]
pub trait MemoryRetriever: Send + Sync {
    /// # Errors
    /// Errores de almacenamiento (un fallo de embeddings degrada a búsqueda léxica, no falla).
    async fn retrieve_context(&self, query: &str, options: RetrievalOptions) -> MemoryResult<RetrievalResult>;
}

/// Experiencias y habilidades.
pub trait ProceduralMemory: Send + Sync {
    /// Guarda la experiencia o, si ya existe una igual, la agrupa. Devuelve el id final.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    fn save_experience(&self, experience: &Experience) -> MemoryResult<String>;
    /// # Errors
    /// Errores de almacenamiento.
    fn list_experiences(&self, limit: usize) -> MemoryResult<Vec<Experience>>;
    /// Convierte una experiencia verificada y completada en habilidad.
    ///
    /// # Errors
    /// [`MemoryError::NotVerified`] si no cumple.
    fn promote(&self, experience_id: &str) -> MemoryResult<Skill>;
}
