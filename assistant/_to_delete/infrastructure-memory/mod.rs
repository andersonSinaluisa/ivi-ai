//! MemoryEngine: memoria episódica, semántica y procedimental sobre SQLite, más un índice
//! vectorial intercambiable.

pub mod secrets;
pub mod sqlite_store;
pub mod vector;

use thiserror::Error;

use crate::domain::{ChatMessage, Experience, MemoryKind, MemoryRecord};

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("el contenido parece contener un secreto (contraseña, token o clave) y no se guarda")]
    ContainsSecret,
    #[error("recuerdo no encontrado")]
    NotFound,
    #[error("la experiencia no está verificada y no puede convertirse en habilidad")]
    NotVerified,
    #[error("almacenamiento: {0}")]
    Storage(String),
}

pub type MemoryResult<T> = Result<T, MemoryError>;

/// Lo que pasó en cada conversación.
pub trait EpisodicMemory: Send + Sync {
    fn record_message(&self, conversation_id: &str, message: &ChatMessage) -> MemoryResult<()>;
    fn recent(&self, conversation_id: &str, limit: usize) -> MemoryResult<Vec<ChatMessage>>;
}

/// Hechos estables (preferencias, datos del usuario) que el usuario puede ver y editar.
pub trait SemanticMemory: Send + Sync {
    fn add(&self, record: MemoryRecord) -> MemoryResult<MemoryRecord>;
    fn update(&self, id: &str, content: &str) -> MemoryResult<MemoryRecord>;
    fn delete(&self, id: &str) -> MemoryResult<()>;
    fn list(&self, kind: Option<MemoryKind>, query: Option<&str>, limit: usize) -> MemoryResult<Vec<MemoryRecord>>;
}

/// Experiencias y habilidades.
pub trait ProceduralMemory: Send + Sync {
    fn save_experience(&self, experience: &Experience) -> MemoryResult<()>;
    fn list_experiences(&self, limit: usize) -> MemoryResult<Vec<Experience>>;
    /// Convierte una experiencia verificada en habilidad. Falla si no está verificada.
    fn promote(&self, id: &str) -> MemoryResult<Experience>;
}

/// Recupera el contexto relevante para una instrucción.
pub trait MemoryRetriever: Send + Sync {
    fn retrieve(&self, query: &str, limit: usize) -> MemoryResult<Vec<MemoryRecord>>;
}

/// Mantenimiento: aplica las políticas de retención.
pub trait MemoryConsolidator: Send + Sync {
    /// Devuelve cuántos recuerdos se eliminaron.
    fn consolidate(&self) -> MemoryResult<usize>;
}
