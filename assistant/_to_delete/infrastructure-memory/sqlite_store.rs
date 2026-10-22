//! Implementación de las memorias sobre la base SQLite local.
//!
//! La recuperación es por coincidencia de palabras clave ponderada por relevancia y confianza.
//! El índice vectorial (`vector.rs`) está listo para enchufarse cuando haya un proveedor de
//! embeddings; mientras tanto no se usa (ver PROGRESS.md).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::secrets::contains_secret;
use super::{
    EpisodicMemory, MemoryConsolidator, MemoryError, MemoryResult, MemoryRetriever, ProceduralMemory, SemanticMemory,
};
use crate::domain::{ChatMessage, Experience, MemoryKind, MemoryRecord, RetentionPolicy};
use crate::infrastructure::persistence::Database;

#[derive(Clone)]
pub struct SqliteMemory {
    db: Database,
}

impl SqliteMemory {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn storage<E: std::fmt::Display>(e: E) -> MemoryError {
    MemoryError::Storage(e.to_string())
}

fn row_to_record(r: &Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let kind: String = r.get(1)?;
    let created: String = r.get(3)?;
    let refs: String = r.get(7)?;
    let retention: String = r.get(8)?;
    Ok(MemoryRecord {
        id: r.get(0)?,
        kind: MemoryKind::parse(&kind).unwrap_or(MemoryKind::Semantic),
        content: r.get(2)?,
        created_at: DateTime::parse_from_rfc3339(&created).map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc)),
        source: r.get(4)?,
        relevance: r.get(5)?,
        confidence: r.get(6)?,
        references: serde_json::from_str(&refs).unwrap_or_default(),
        retention: serde_json::from_str(&retention).unwrap_or(RetentionPolicy::Permanent),
    })
}

const SELECT: &str = "SELECT id, kind, content, created_at, source, relevance, confidence, refs, retention FROM memories";

fn tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_owned)
        .collect()
}

impl EpisodicMemory for SqliteMemory {
    fn record_message(&self, conversation_id: &str, message: &ChatMessage) -> MemoryResult<()> {
        self.db.append_message(conversation_id, message).map(|_| ()).map_err(storage)
    }

    fn recent(&self, conversation_id: &str, limit: usize) -> MemoryResult<Vec<ChatMessage>> {
        Ok(self
            .db
            .recent_messages(conversation_id, limit)
            .map_err(storage)?
            .iter()
            .filter_map(crate::infrastructure::persistence::conversations::StoredMessage::to_chat)
            .collect())
    }
}

impl SemanticMemory for SqliteMemory {
    fn add(&self, mut record: MemoryRecord) -> MemoryResult<MemoryRecord> {
        if contains_secret(&record.content) {
            return Err(MemoryError::ContainsSecret);
        }
        if record.id.is_empty() {
            record.id = uuid::Uuid::new_v4().to_string();
        }
        record.relevance = record.relevance.clamp(0.0, 1.0);
        record.confidence = record.confidence.clamp(0.0, 1.0);
        self.db
            .conn()
            .execute(
                "INSERT INTO memories (id, kind, content, created_at, source, relevance, confidence, refs, retention)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.id,
                    record.kind.as_str(),
                    record.content,
                    record.created_at.to_rfc3339(),
                    record.source,
                    record.relevance,
                    record.confidence,
                    serde_json::to_string(&record.references).map_err(storage)?,
                    serde_json::to_string(&record.retention).map_err(storage)?,
                ],
            )
            .map_err(storage)?;
        Ok(record)
    }

    fn update(&self, id: &str, content: &str) -> MemoryResult<MemoryRecord> {
        if contains_secret(content) {
            return Err(MemoryError::ContainsSecret);
        }
        let conn = self.db.conn();
        let n = conn.execute("UPDATE memories SET content = ?2 WHERE id = ?1", params![id, content]).map_err(storage)?;
        if n == 0 {
            return Err(MemoryError::NotFound);
        }
        conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], row_to_record).map_err(storage)
    }

    fn delete(&self, id: &str) -> MemoryResult<()> {
        let n = self.db.conn().execute("DELETE FROM memories WHERE id = ?1", [id]).map_err(storage)?;
        if n == 0 {
            Err(MemoryError::NotFound)
        } else {
            Ok(())
        }
    }

    fn list(&self, kind: Option<MemoryKind>, query: Option<&str>, limit: usize) -> MemoryResult<Vec<MemoryRecord>> {
        let conn = self.db.conn();
        let like = query.filter(|q| !q.trim().is_empty()).map(|q| format!("%{}%", q.trim()));
        let mut stmt = conn
            .prepare(&format!(
                "{SELECT} WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR content LIKE ?2)
                 ORDER BY created_at DESC LIMIT ?3"
            ))
            .map_err(storage)?;
        let rows = stmt
            .query_map(
                params![kind.map(MemoryKind::as_str), like, i64::try_from(limit).unwrap_or(i64::MAX)],
                row_to_record,
            )
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }
}

impl ProceduralMemory for SqliteMemory {
    fn save_experience(&self, e: &Experience) -> MemoryResult<()> {
        self.db
            .conn()
            .execute(
                "INSERT OR REPLACE INTO experiences (id, data, verified, promoted, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![e.id, serde_json::to_string(e).map_err(storage)?, e.verified, e.promoted, e.created_at.to_rfc3339()],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn list_experiences(&self, limit: usize) -> MemoryResult<Vec<Experience>> {
        let conn = self.db.conn();
        let mut stmt =
            conn.prepare("SELECT data FROM experiences ORDER BY created_at DESC LIMIT ?1").map_err(storage)?;
        let rows = stmt
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get::<_, String>(0))
            .map_err(storage)?;
        let mut out = Vec::new();
        for data in rows {
            if let Ok(e) = serde_json::from_str::<Experience>(&data.map_err(storage)?) {
                out.push(e);
            }
        }
        Ok(out)
    }

    fn promote(&self, id: &str) -> MemoryResult<Experience> {
        let data: Option<String> = self
            .db
            .conn()
            .query_row("SELECT data FROM experiences WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        let mut e: Experience = serde_json::from_str(&data.ok_or(MemoryError::NotFound)?).map_err(storage)?;
        if !e.verified || e.procedure.is_none() {
            return Err(MemoryError::NotVerified);
        }
        e.promoted = true;
        self.save_experience(&e)?;
        // La habilidad queda también como memoria procedimental consultable.
        self.add(MemoryRecord {
            id: String::new(),
            kind: MemoryKind::Procedural,
            content: format!("Para \"{}\":\n{}", e.goal, e.procedure.clone().unwrap_or_default()),
            created_at: Utc::now(),
            source: format!("experience:{}", e.id),
            relevance: 0.7,
            confidence: 0.9,
            references: vec![e.id.clone()],
            retention: RetentionPolicy::Permanent,
        })?;
        Ok(e)
    }
}

impl MemoryRetriever for SqliteMemory {
    fn retrieve(&self, query: &str, limit: usize) -> MemoryResult<Vec<MemoryRecord>> {
        let q = tokens(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = {
            let conn = self.db.conn();
            let mut stmt = conn
                .prepare(&format!("{SELECT} WHERE kind IN ('semantic', 'procedural') ORDER BY created_at DESC LIMIT 500"))
                .map_err(storage)?;
            let rows = stmt.query_map([], row_to_record).map_err(storage)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage)?
        };
        let now = Utc::now();
        let mut scored: Vec<(f32, MemoryRecord)> = candidates
            .into_iter()
            .filter(|r| !r.is_expired(now))
            .filter_map(|r| {
                let overlap = tokens(&r.content).intersection(&q).count();
                #[allow(clippy::cast_precision_loss)]
                let score = overlap as f32 * (0.5 + r.relevance) * (0.5 + r.confidence);
                (overlap > 0).then_some((score, r))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        Ok(scored.into_iter().take(limit).map(|(_, r)| r).collect())
    }
}

impl MemoryConsolidator for SqliteMemory {
    fn consolidate(&self) -> MemoryResult<usize> {
        let all = self.list(None, None, 100_000)?;
        let now = Utc::now();
        let mut removed = 0;
        for r in all.iter().filter(|r| r.is_expired(now)) {
            self.delete(&r.id)?;
            removed += 1;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ExperienceStep, VerificationStatus};
    use serde_json::json;

    fn record(content: &str, retention: RetentionPolicy) -> MemoryRecord {
        MemoryRecord {
            id: String::new(),
            kind: MemoryKind::Semantic,
            content: content.into(),
            created_at: Utc::now(),
            source: "user".into(),
            relevance: 0.5,
            confidence: 0.8,
            references: vec![],
            retention,
        }
    }

    #[test]
    fn crud_and_retrieval_work() {
        let m = SqliteMemory::new(Database::open_in_memory().expect("db"));
        let a = m.add(record("El usuario vive en Guayaquil", RetentionPolicy::Permanent)).expect("a");
        m.add(record("Prefiere respuestas cortas", RetentionPolicy::Permanent)).expect("b");

        let hits = m.retrieve("¿qué clima hace en guayaquil?", 5).expect("r");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, a.id);

        let updated = m.update(&a.id, "El usuario vive en Quito").expect("upd");
        assert_eq!(updated.content, "El usuario vive en Quito");
        m.delete(&a.id).expect("del");
        assert!(matches!(m.delete(&a.id), Err(MemoryError::NotFound)));
        assert_eq!(m.list(Some(MemoryKind::Semantic), None, 10).expect("list").len(), 1);
    }

    #[test]
    fn refuses_to_store_secrets() {
        let m = SqliteMemory::new(Database::open_in_memory().expect("db"));
        assert!(matches!(m.add(record("mi password: abc123", RetentionPolicy::Permanent)), Err(MemoryError::ContainsSecret)));
        let ok = m.add(record("color favorito azul", RetentionPolicy::Permanent)).expect("ok");
        assert!(matches!(m.update(&ok.id, "token: xyz"), Err(MemoryError::ContainsSecret)));
    }

    #[test]
    fn consolidation_applies_retention() {
        let m = SqliteMemory::new(Database::open_in_memory().expect("db"));
        m.add(record("temporal", RetentionPolicy::Session)).expect("a");
        let mut old = record("viejo", RetentionPolicy::Days { days: 1 });
        old.created_at = Utc::now() - chrono::Duration::days(3);
        m.add(old).expect("b");
        m.add(record("permanente", RetentionPolicy::Permanent)).expect("c");
        assert_eq!(m.consolidate().expect("c"), 2);
        assert_eq!(m.list(None, None, 10).expect("l").len(), 1);
    }

    #[test]
    fn only_verified_experiences_can_be_promoted() {
        let m = SqliteMemory::new(Database::open_in_memory().expect("db"));
        let step = |v| ExperienceStep { tool: "get_current_time".into(), arguments: json!({}), ok: true, verification: v };
        let mk = |id: &str, steps: Vec<ExperienceStep>| {
            let procedure = Experience::derive_procedure(&steps);
            Experience {
                id: id.into(),
                goal: "saber la hora".into(),
                context: String::new(),
                plan: String::new(),
                verified: procedure.is_some(),
                procedure,
                steps,
                result: "ok".into(),
                evidence: vec![],
                errors: vec![],
                promoted: false,
                created_at: Utc::now(),
            }
        };
        m.save_experience(&mk("e1", vec![step(VerificationStatus::Passed)])).expect("e1");
        m.save_experience(&mk("e2", vec![step(VerificationStatus::Inconclusive)])).expect("e2");

        assert!(m.promote("e1").expect("promote").promoted);
        assert!(matches!(m.promote("e2"), Err(MemoryError::NotVerified)));
        assert_eq!(m.list(Some(MemoryKind::Procedural), None, 10).expect("l").len(), 1);
    }
}
