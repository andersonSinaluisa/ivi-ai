//! Memoria episódica: el historial de conversaciones (tablas `conversations` y `messages`).

use super::memory_repository::storage;
use super::{EpisodicMemory, MemoryResult};
use crate::domain::ChatMessage;
use crate::infrastructure::persistence::conversations::StoredMessage;
use crate::infrastructure::persistence::Database;

#[derive(Clone)]
pub struct EpisodicStore {
    db: Database,
}

impl EpisodicStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

impl EpisodicMemory for EpisodicStore {
    fn record_message(&self, conversation_id: &str, message: &ChatMessage) -> MemoryResult<()> {
        self.db.append_message(conversation_id, message).map(|_| ()).map_err(storage)
    }

    fn recent(&self, conversation_id: &str, limit: usize) -> MemoryResult<Vec<ChatMessage>> {
        Ok(self.db.recent_messages(conversation_id, limit).map_err(storage)?.iter().filter_map(StoredMessage::to_chat).collect())
    }
}
