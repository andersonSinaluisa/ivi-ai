//! Historial local de conversaciones (memoria episódica en bruto).

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::{Database, DbResult};
use crate::domain::{ChatMessage, Role, ToolCall};
use crate::memory::secrets::redact_secrets;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub created_at: String,
    #[serde(skip)]
    pub tool_calls: Vec<ToolCall>,
}

impl StoredMessage {
    #[must_use]
    pub fn to_chat(&self) -> Option<ChatMessage> {
        Some(ChatMessage {
            role: Role::parse(&self.role)?,
            content: self.content.clone(),
            tool_calls: self.tool_calls.clone(),
            tool_name: self.tool_name.clone(),
        })
    }
}

fn title_from(text: &str) -> String {
    let t: String = text.trim().chars().take(60).collect();
    if t.is_empty() { "Conversación".into() } else { t }
}

impl Database {
    /// Devuelve la conversación existente o crea una nueva titulada con el primer mensaje.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn ensure_conversation(&self, id: Option<&str>, first_text: &str) -> DbResult<String> {
        let conn = self.conn();
        if let Some(id) = id {
            let exists: Option<String> =
                conn.query_row("SELECT id FROM conversations WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            if let Some(id) = exists {
                return Ok(id);
            }
        }
        let id = id.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO conversations (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![id, title_from(&redact_secrets(first_text)), now],
        )?;
        Ok(id)
    }

    /// Guarda un mensaje. Los secretos se redactan antes de tocar el disco.
    ///
    /// # Errors
    /// Errores de SQLite o JSON.
    pub fn append_message(&self, conversation_id: &str, message: &ChatMessage) -> DbResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let calls = if message.tool_calls.is_empty() { None } else { Some(serde_json::to_string(&message.tool_calls)?) };
        let conn = self.conn();
        conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, tool_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, conversation_id, message.role.as_str(), redact_secrets(&message.content), calls, message.tool_name, now],
        )?;
        conn.execute("UPDATE conversations SET updated_at = ?2 WHERE id = ?1", params![conversation_id, now])?;
        Ok(id)
    }

    /// Últimos `limit` mensajes, en orden cronológico.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn recent_messages(&self, conversation_id: &str, limit: usize) -> DbResult<Vec<StoredMessage>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, role, content, tool_name, created_at, tool_calls FROM messages
             WHERE conversation_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![conversation_id, i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            let calls: Option<String> = r.get(6)?;
            Ok(StoredMessage {
                id: r.get(0)?,
                conversation_id: r.get(1)?,
                role: r.get(2)?,
                content: r.get(3)?,
                tool_name: r.get(4)?,
                created_at: r.get(5)?,
                tool_calls: calls.and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default(),
            })
        })?;
        let mut out: Vec<StoredMessage> = rows.collect::<Result<_, _>>()?;
        out.reverse();
        Ok(out)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn list_conversations(&self, limit: usize) -> DbResult<Vec<Conversation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at FROM conversations ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            Ok(Conversation { id: r.get(0)?, title: r.get(1)?, created_at: r.get(2)?, updated_at: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn delete_conversation(&self, id: &str) -> DbResult<bool> {
        Ok(self.conn().execute("DELETE FROM conversations WHERE id = ?1", [id])? > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_persists_in_order_and_survives_reopen() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("j.db");
        let conv;
        {
            let db = Database::open(&path).expect("open");
            conv = db.ensure_conversation(None, "¿Qué hora es?").expect("conv");
            db.append_message(&conv, &ChatMessage::user("¿Qué hora es?")).expect("m1");
            db.append_message(&conv, &ChatMessage::assistant("Son las 10:00.")).expect("m2");
        }
        let db = Database::open(&path).expect("reopen");
        let msgs = db.recent_messages(&conv, 10).expect("msgs");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].content, "Son las 10:00.");
        let convs = db.list_conversations(10).expect("list");
        assert_eq!(convs[0].title, "¿Qué hora es?");
        assert_eq!(db.ensure_conversation(Some(&conv), "otro").expect("same"), conv);
    }

    #[test]
    fn secrets_are_redacted_before_storage() {
        let db = Database::open_in_memory().expect("db");
        let conv = db.ensure_conversation(None, "x").expect("conv");
        db.append_message(&conv, &ChatMessage::user("mi password: Sup3rS3creta!")).expect("m");
        let msgs = db.recent_messages(&conv, 1).expect("msgs");
        assert!(!msgs[0].content.contains("Sup3rS3creta"), "{}", msgs[0].content);
    }

    #[test]
    fn deleting_a_conversation_deletes_its_messages() {
        let db = Database::open_in_memory().expect("db");
        let conv = db.ensure_conversation(None, "x").expect("conv");
        db.append_message(&conv, &ChatMessage::user("hola")).expect("m");
        assert!(db.delete_conversation(&conv).expect("del"));
        assert!(db.recent_messages(&conv, 10).expect("msgs").is_empty());
    }
}
