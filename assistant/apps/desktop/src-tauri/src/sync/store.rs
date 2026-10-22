//! Estado local de la sincronización en SQLite (migración 3).
//!
//! - `sync_outbox` la llenan **triggers** en la misma transacción que cada cambio local (alta, edición o borrado de
//!   recuerdos, relaciones, experiencias, habilidades y vectores de modelos reales). Solo cuando la sincronización
//!   está activada (`sync_meta.enabled`).
//! - Los cambios recibidos del Cloud se aplican con `sync_meta.suppress` dentro de la transacción: los triggers no
//!   los vuelven a encolar.
//! - `sync_state` guarda la versión del servidor sobre la que está cada registro local (la `baseVersion` del push).

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::protocol::{
    content_sha256, entity, status, Change, EmbeddingData, ExperienceData, ExperienceStepData, MemoryData, PulledChange, PushResult,
    RelationData, RetentionData, SkillCompatibility, SkillData, SkillPermissions, SourceData,
};
use crate::domain::memory::{RetentionPolicy, SourceKind};
use crate::domain::{Experience, ExperienceStep, VerificationStatus};
use crate::infrastructure::persistence::Database;
use crate::memory::memory_repository::{content_hash, row_to_memory, MEMORY_COLUMNS};

pub type StoreResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Lote preparado para un push: cambios y, por cada changeId, la última fila del outbox que cubre.
#[derive(Debug, Default)]
pub struct Batch {
    pub changes: Vec<Change>,
    /// changeId → (entity, entity_id, detail, seq máximo cubierto).
    pub covers: HashMap<String, (String, String, Option<String>, i64)>,
}

/// Resultado de aplicar cambios recibidos: qué recuerdos cambiaron (para el Neural Core).
#[derive(Debug, Default, Clone)]
pub struct Applied {
    pub upserted_memories: Vec<String>,
    pub deleted_memories: Vec<String>,
    pub relations: usize,
    pub skills: usize,
    pub experiences: usize,
    pub embeddings: usize,
    pub skipped_pending: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalConflict {
    pub id: String,
    pub entity: String,
    pub entity_id: String,
    pub kind: String,
    pub fields: Vec<String>,
    pub server_version: i64,
    pub server_data: Option<Value>,
    pub device_data: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxStats {
    pub pending: u64,
    pub rejected: u64,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct SyncStore {
    db: Database,
}

/// Clave de orden de un grupo del outbox: (prioridad de entidad, primer seq, entidad, id, detalle).
type GroupKey = (u8, i64, String, String, Option<String>);

fn entity_order(entity: &str) -> u8 {
    match entity {
        entity::MEMORY => 0,
        entity::EMBEDDING => 1,
        entity::RELATION => 2,
        entity::EXPERIENCE => 3,
        _ => 4,
    }
}

impl SyncStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    // ------------------------------------------------------------------ meta

    /// # Errors
    /// Errores de SQLite.
    pub fn meta(&self, key: &str) -> StoreResult<Option<String>> {
        self.db.conn().query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(err)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn set_meta(&self, key: &str, value: &str) -> StoreResult<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO sync_meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(err)?;
        Ok(())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn delete_meta(&self, key: &str) -> StoreResult<()> {
        self.db.conn().execute("DELETE FROM sync_meta WHERE key = ?1", [key]).map_err(err)?;
        Ok(())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn is_enabled(&self) -> StoreResult<bool> {
        Ok(self.meta("enabled")?.is_some())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn cursor(&self) -> StoreResult<i64> {
        Ok(self.meta("cursor")?.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    /// Activa la captura y encola todo lo existente (primer push tras emparejar).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn enable_and_enqueue_all(&self) -> StoreResult<u64> {
        let mut conn = self.db.conn();
        let tx = conn.transaction().map_err(err)?;
        tx.execute("INSERT OR IGNORE INTO sync_meta (key, value) VALUES ('enabled', '1')", []).map_err(err)?;
        let uuid = "lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6)))";
        let ts = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
        let mut total = 0u64;
        for (entity, table, filter, detail) in [
            (entity::MEMORY, "memories", "retention NOT LIKE '%session%'", "NULL"),
            (entity::EMBEDDING, "embedding_metadata", "model NOT LIKE 'lexical-%'", "model"),
            (entity::RELATION, "memory_relations", "1 = 1", "NULL"),
            (entity::EXPERIENCE, "experiences", "1 = 1", "NULL"),
            (entity::SKILL, "skills", "1 = 1", "NULL"),
        ] {
            let id = if entity == entity::EMBEDDING { "memory_id" } else { "id" };
            total += tx
                .execute(
                    &format!(
                        "INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
                         SELECT {uuid}, '{entity}', {id}, 'upsert', {detail}, {ts} FROM {table} WHERE {filter}"
                    ),
                    [],
                )
                .map_err(err)? as u64;
        }
        tx.commit().map_err(err)?;
        Ok(total)
    }

    /// Desactiva la sincronización y olvida su estado local (no toca recuerdos).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn disable_and_forget(&self) -> StoreResult<()> {
        self.db
            .conn()
            .execute_batch(
                "BEGIN; DELETE FROM sync_outbox; DELETE FROM sync_state; DELETE FROM sync_conflicts; DELETE FROM sync_meta; COMMIT;",
            )
            .map_err(err)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn outbox_stats(&self) -> StoreResult<OutboxStats> {
        let conn = self.db.conn();
        let count = |status: &str| -> StoreResult<u64> {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM (SELECT DISTINCT entity, entity_id, detail FROM sync_outbox WHERE status = ?1)",
                    [status],
                    |r| r.get(0),
                )
                .map_err(err)?;
            Ok(u64::try_from(n).unwrap_or(0))
        };
        let last_error = conn
            .query_row("SELECT last_error FROM sync_outbox WHERE last_error IS NOT NULL ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
            .optional()
            .map_err(err)?;
        Ok(OutboxStats { pending: count("pending")?, rejected: count("rejected")?, last_error })
    }

    // ------------------------------------------------------------------ push

    /// Agrupa el outbox por registro y arma los cambios con el **estado actual** de cada uno.
    ///
    /// # Errors
    /// Errores de SQLite o de serialización.
    pub fn next_batch(&self, max_changes: usize, max_bytes: usize) -> StoreResult<Batch> {
        let conn = self.db.conn();
        let mut groups: BTreeMap<GroupKey, (String, i64)> = BTreeMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT g.entity, g.entity_id, g.detail, g.min_seq, g.max_seq, o.change_id
                     FROM (SELECT entity, entity_id, detail, MIN(seq) AS min_seq, MAX(seq) AS max_seq
                           FROM sync_outbox WHERE status = 'pending' GROUP BY entity, entity_id, detail) g
                     JOIN sync_outbox o ON o.seq = g.max_seq",
                )
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })
                .map_err(err)?;
            for row in rows {
                let (entity, id, detail, min_seq, max_seq, change_id) = row.map_err(err)?;
                groups.insert((entity_order(&entity), min_seq, entity, id, detail), (change_id, max_seq));
            }
        }

        let mut batch = Batch::default();
        let mut bytes = 0usize;
        for ((_, _, entity, id, detail), (change_id, max_seq)) in groups {
            if batch.changes.len() >= max_changes {
                break;
            }
            let base = base_version(&conn, &entity, &id)?;
            let data = current_data(&conn, &entity, &id, detail.as_deref())?;
            let change = Change {
                change_id: change_id.clone(),
                entity: entity.clone(),
                entity_id: id.clone(),
                op: if data.is_some() { "upsert".into() } else { "delete".into() },
                base_version: base,
                data,
            };
            if entity == entity::EMBEDDING && change.data.is_none() {
                // El vector ya no existe (o el contenido cambió): nada que enviar.
                drop_group(&conn, &entity, &id, detail.as_deref(), max_seq)?;
                continue;
            }
            let size = serde_json::to_vec(&change).map_err(err)?.len();
            if !batch.changes.is_empty() && bytes + size > max_bytes {
                break;
            }
            bytes += size;
            batch.covers.insert(change_id, (entity, id, detail, max_seq));
            batch.changes.push(change);
        }
        Ok(batch)
    }

    /// Aplica las respuestas del push. Devuelve los ids de recuerdos que cambiaron por una fusión.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn acknowledge(&self, batch: &Batch, results: &[PushResult]) -> StoreResult<Applied> {
        let mut applied = Applied::default();
        let mut conn = self.db.conn();
        let tx = conn.transaction().map_err(err)?;
        tx.execute("INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('suppress', '1')", []).map_err(err)?;
        for result in results {
            let Some((entity, id, detail, max_seq)) = batch.covers.get(&result.change_id) else { continue };
            match result.status.as_str() {
                status::APPLIED | status::MERGED => {
                    if let Some(version) = result.version.filter(|v| *v > 0) {
                        if entity != entity::EMBEDDING {
                            set_state(&tx, entity, id, version)?;
                        }
                    }
                    if result.status == status::MERGED {
                        if let Some(data) = &result.data {
                            apply_upsert(&tx, entity, id, data, &mut applied)?;
                        }
                    }
                    drop_group(&tx, entity, id, detail.as_deref(), *max_seq)?;
                }
                status::CONFLICT => {
                    let device = batch.changes.iter().find(|c| c.change_id == result.change_id).and_then(|c| c.data.clone());
                    let conflict_id = result.conflict_id.clone().unwrap_or_else(|| result.change_id.clone());
                    tx.execute(
                        "INSERT OR REPLACE INTO sync_conflicts (id, entity, entity_id, kind, fields, server_version, server_data, device_data, status, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9)",
                        params![
                            conflict_id,
                            entity,
                            id,
                            result.conflict_kind.clone().unwrap_or_default(),
                            serde_json::to_string(&result.fields.clone().unwrap_or_default()).map_err(err)?,
                            result.version.unwrap_or(0),
                            result.data.as_ref().map(Value::to_string),
                            device.as_ref().map(Value::to_string),
                            now()
                        ],
                    )
                    .map_err(err)?;
                    tx.execute(
                        "UPDATE sync_state SET conflict_id = ?3 WHERE entity = ?1 AND entity_id = ?2",
                        params![entity, id, conflict_id],
                    )
                    .map_err(err)?;
                    drop_group(&tx, entity, id, detail.as_deref(), *max_seq)?;
                }
                _ => {
                    let message = format!("{}: {}", result.code.clone().unwrap_or_default(), result.message.clone().unwrap_or_default());
                    tx.execute(
                        "UPDATE sync_outbox SET status = 'rejected', last_error = ?4
                         WHERE entity = ?1 AND entity_id = ?2 AND seq <= ?3",
                        params![entity, id, max_seq, message],
                    )
                    .map_err(err)?;
                }
            }
        }
        tx.execute("DELETE FROM sync_meta WHERE key = 'suppress'", []).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(applied)
    }

    /// Marca un intento fallido (error de red) sin perder nada.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn record_attempt_error(&self, batch: &Batch, error: &str) -> StoreResult<()> {
        let conn = self.db.conn();
        for (entity, id, _, max_seq) in batch.covers.values() {
            conn.execute(
                "UPDATE sync_outbox SET attempts = attempts + 1, last_error = ?4 WHERE entity = ?1 AND entity_id = ?2 AND seq <= ?3",
                params![entity, id, max_seq, error],
            )
            .map_err(err)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ pull

    /// Aplica un lote recibido. Las relaciones cuyos extremos aún no existen se devuelven para reintentarlas al final
    /// de la sesión (un recuerdo editado después de crear la relación llega con un seq posterior).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn apply_pulled(&self, changes: &[PulledChange], local_model: Option<&str>, applied: &mut Applied) -> StoreResult<Vec<PulledChange>> {
        let mut deferred = Vec::new();
        let mut conn = self.db.conn();
        let tx = conn.transaction().map_err(err)?;
        tx.execute("INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('suppress', '1')", []).map_err(err)?;
        for change in changes {
            if change.entity == entity::EMBEDDING {
                if let (Some(model), Some(data)) = (local_model, &change.data) {
                    apply_embedding(&tx, &change.entity_id, model, data, applied)?;
                }
                continue;
            }
            if let Some(current) = base_version(&tx, &change.entity, &change.entity_id)? {
                if current >= change.version {
                    continue;
                }
            }
            // Conflicto abierto: la copia local se conserva hasta que llegue una versión posterior (la resolución).
            let conflicted: Option<i64> = tx
                .query_row(
                    "SELECT MAX(server_version) FROM sync_conflicts WHERE entity = ?1 AND entity_id = ?2 AND status = 'open'",
                    params![change.entity, change.entity_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if conflicted.is_some_and(|v| change.version <= v) {
                applied.skipped_pending += 1;
                continue;
            }
            let pending: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE entity = ?1 AND entity_id = ?2 AND status = 'pending'",
                    params![change.entity, change.entity_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if pending > 0 {
                // Cambio local sin enviar: no se pisa; el push siguiente se fusiona o genera un conflicto en el Cloud.
                applied.skipped_pending += 1;
                continue;
            }
            let done = if change.op == "delete" {
                apply_delete(&tx, &change.entity, &change.entity_id, applied)?;
                true
            } else if let Some(data) = &change.data {
                apply_upsert(&tx, &change.entity, &change.entity_id, data, applied)?
            } else {
                true
            };
            if done {
                set_state(&tx, &change.entity, &change.entity_id, change.version)?;
                tx.execute(
                    "UPDATE sync_conflicts SET status = 'superseded' WHERE entity = ?1 AND entity_id = ?2 AND status = 'open' AND server_version < ?3",
                    params![change.entity, change.entity_id, change.version],
                )
                .map_err(err)?;
            } else {
                deferred.push(change.clone());
            }
        }
        tx.execute("DELETE FROM sync_meta WHERE key = 'suppress'", []).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(deferred)
    }

    // ------------------------------------------------------------------ conflictos

    /// # Errors
    /// Errores de SQLite.
    pub fn open_conflicts(&self) -> StoreResult<Vec<LocalConflict>> {
        let conn = self.db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, entity, entity_id, kind, fields, server_version, server_data, device_data, created_at
                 FROM sync_conflicts WHERE status = 'open' ORDER BY created_at DESC",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                let fields: String = r.get(4)?;
                let server: Option<String> = r.get(6)?;
                let device: Option<String> = r.get(7)?;
                Ok(LocalConflict {
                    id: r.get(0)?,
                    entity: r.get(1)?,
                    entity_id: r.get(2)?,
                    kind: r.get(3)?,
                    fields: serde_json::from_str(&fields).unwrap_or_default(),
                    server_version: r.get(5)?,
                    server_data: server.and_then(|s| serde_json::from_str(&s).ok()),
                    device_data: device.and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: r.get(8)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn mark_conflict(&self, id: &str, status: &str) -> StoreResult<()> {
        self.db.conn().execute("UPDATE sync_conflicts SET status = ?2 WHERE id = ?1", params![id, status]).map_err(err)?;
        Ok(())
    }

    /// Recuerdos locales y cuántos de ellos ya tienen versión en el Cloud.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn memory_counts(&self) -> StoreResult<(u64, u64)> {
        let conn = self.db.conn();
        let local: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0)).map_err(err)?;
        let synced: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories m JOIN sync_state s ON s.entity = 'memory' AND s.entity_id = m.id
                 WHERE NOT EXISTS (SELECT 1 FROM sync_outbox o WHERE o.entity = 'memory' AND o.entity_id = m.id AND o.status = 'pending')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        Ok((u64::try_from(local).unwrap_or(0), u64::try_from(synced).unwrap_or(0)))
    }

    /// Versión del servidor conocida para un registro (None = nunca sincronizado).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn server_version(&self, entity: &str, id: &str) -> StoreResult<Option<i64>> {
        base_version(&self.db.conn(), entity, id)
    }
}

fn base_version(conn: &Connection, entity: &str, id: &str) -> StoreResult<Option<i64>> {
    conn.query_row("SELECT server_version FROM sync_state WHERE entity = ?1 AND entity_id = ?2", params![entity, id], |r| r.get(0))
        .optional()
        .map_err(err)
}

fn set_state(conn: &Connection, entity: &str, id: &str, version: i64) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO sync_state (entity, entity_id, server_version, synced_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (entity, entity_id) DO UPDATE SET server_version = MAX(server_version, excluded.server_version), synced_at = excluded.synced_at",
        params![entity, id, version, now()],
    )
    .map_err(err)?;
    Ok(())
}

fn drop_group(conn: &Connection, entity: &str, id: &str, detail: Option<&str>, max_seq: i64) -> StoreResult<()> {
    conn.execute(
        "DELETE FROM sync_outbox WHERE entity = ?1 AND entity_id = ?2 AND (detail IS ?3) AND seq <= ?4",
        params![entity, id, detail, max_seq],
    )
    .map_err(err)?;
    Ok(())
}

// ---------------------------------------------------------------------- local → cable

fn current_data(conn: &Connection, entity: &str, id: &str, detail: Option<&str>) -> StoreResult<Option<Value>> {
    let value = match entity {
        entity::MEMORY => {
            let Some(memory) = conn
                .query_row(&format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = ?1"), [id], row_to_memory)
                .optional()
                .map_err(err)?
            else {
                return Ok(None);
            };
            let source = memory
                .source_id
                .as_ref()
                .and_then(|sid| {
                    conn.query_row("SELECT kind, reference, label FROM memory_sources WHERE id = ?1", [sid], |r| {
                        Ok(SourceData { kind: r.get(0)?, reference: r.get(1)?, label: r.get(2)? })
                    })
                    .optional()
                    .ok()
                    .flatten()
                })
                .unwrap_or(SourceData { kind: "user".into(), reference: None, label: Some(memory.source.clone()) });
            serde_json::to_value(MemoryData {
                kind: memory.kind.as_str().into(),
                content: memory.content,
                summary: memory.summary,
                tags: memory.tags,
                relevance: memory.relevance.clamp(0.0, 1.0),
                confidence: memory.confidence.clamp(0.0, 1.0),
                verification: memory.verification.as_str().into(),
                retention: match memory.retention {
                    RetentionPolicy::Days { days } => RetentionData { kind: "days".into(), days: Some(days.max(1)) },
                    _ => RetentionData { kind: "permanent".into(), days: None },
                },
                source,
                created_at: memory.created_at,
                updated_at: memory.updated_at,
            })
            .map_err(err)?
        }
        entity::RELATION => {
            let Some(relation) = conn
                .query_row(
                    "SELECT source_id, target_id, kind, confidence, provenance, verified, note, created_at FROM memory_relations WHERE id = ?1",
                    [id],
                    |r| {
                        let created: String = r.get(7)?;
                        Ok(RelationData {
                            source_id: r.get(0)?,
                            target_id: r.get(1)?,
                            kind: r.get(2)?,
                            confidence: r.get(3)?,
                            provenance: r.get(4)?,
                            verified: r.get::<_, i64>(5)? != 0,
                            note: r.get(6)?,
                            created_at: DateTime::parse_from_rfc3339(&created).ok().map(|d| d.with_timezone(&Utc)),
                        })
                    },
                )
                .optional()
                .map_err(err)?
            else {
                return Ok(None);
            };
            serde_json::to_value(relation).map_err(err)?
        }
        entity::EMBEDDING => {
            let model = detail.unwrap_or_default();
            let row: Option<(String, Vec<u8>, String)> = conn
                .query_row(
                    "SELECT m.content, e.vector, e.content_hash = m.content_hash FROM embedding_metadata e
                     JOIN memories m ON m.id = e.memory_id WHERE e.memory_id = ?1 AND e.model = ?2",
                    params![id, model],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)?.to_string())),
                )
                .optional()
                .map_err(err)?;
            let Some((content, blob, current)) = row else { return Ok(None) };
            if current != "1" {
                return Ok(None);
            }
            let vector: Vec<f32> = blob.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            serde_json::to_value(EmbeddingData { model: model.into(), dims: vector.len(), content_hash: content_sha256(&content), vector })
                .map_err(err)?
        }
        entity::EXPERIENCE => {
            let Some(data): Option<String> =
                conn.query_row("SELECT data FROM experiences WHERE id = ?1", [id], |r| r.get(0)).optional().map_err(err)?
            else {
                return Ok(None);
            };
            let e: Experience = serde_json::from_str(&data).map_err(err)?;
            serde_json::to_value(ExperienceData {
                signature: e.signature(),
                goal: e.goal,
                context: e.context,
                plan: e.plan,
                steps: e
                    .steps
                    .into_iter()
                    .map(|s| ExperienceStepData {
                        tool: s.tool,
                        arguments: s.arguments,
                        ok: s.ok,
                        verification: serde_json::to_value(s.verification).ok().and_then(|v| v.as_str().map(str::to_owned)),
                    })
                    .collect(),
                result: e.result,
                evidence: e.evidence,
                errors: e.errors,
                procedure: e.procedure,
                verified: e.verified,
                completed: e.completed,
                occurrences: e.occurrences,
                created_at: Some(e.created_at),
            })
            .map_err(err)?
        }
        _ => {
            let Some(skill) = conn
                .query_row(
                    "SELECT name, description, procedure, status, compatibility, permissions, source, experience_id FROM skills WHERE id = ?1",
                    [id],
                    |r| {
                        let compat: String = r.get(4)?;
                        let perms: String = r.get(5)?;
                        Ok(SkillData {
                            name: r.get(0)?,
                            description: r.get(1)?,
                            procedure: r.get(2)?,
                            status: r.get(3)?,
                            compatibility: serde_json::from_str(&compat).ok(),
                            permissions: serde_json::from_str(&perms).ok(),
                            source: Some(r.get::<_, String>(6)?),
                            experience_id: r.get(7)?,
                            changelog: None,
                        })
                    },
                )
                .optional()
                .map_err(err)?
            else {
                return Ok(None);
            };
            serde_json::to_value(skill).map_err(err)?
        }
    };
    Ok(Some(value))
}

// ---------------------------------------------------------------------- cable → local

/// Devuelve false si no se pudo aplicar todavía (relación con extremos ausentes).
fn apply_upsert(conn: &Connection, entity: &str, id: &str, data: &Value, applied: &mut Applied) -> StoreResult<bool> {
    match entity {
        entity::MEMORY => {
            let m: MemoryData = serde_json::from_value(data.clone()).map_err(err)?;
            let retention = match (m.retention.kind.as_str(), m.retention.days) {
                ("days", Some(days)) => RetentionPolicy::Days { days },
                _ => RetentionPolicy::Permanent,
            };
            let exists: bool =
                conn.query_row("SELECT COUNT(*) FROM memories WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).map_err(err)? > 0;
            let tags = serde_json::to_string(&m.tags).map_err(err)?;
            let retention_json = serde_json::to_string(&retention).map_err(err)?;
            if exists {
                conn.execute(
                    "UPDATE memories SET kind = ?2, content = ?3, summary = ?4, tags = ?5, relevance = ?6, confidence = ?7,
                     verification = ?8, retention = ?9, updated_at = ?10, content_hash = ?11 WHERE id = ?1",
                    params![
                        id,
                        m.kind,
                        m.content,
                        m.summary,
                        tags,
                        m.relevance,
                        m.confidence,
                        m.verification,
                        retention_json,
                        m.updated_at.to_rfc3339(),
                        content_hash(&m.content)
                    ],
                )
                .map_err(err)?;
            } else {
                let kind = SourceKind::parse(&m.source.kind).unwrap_or(SourceKind::Device);
                let source_id = uuid::Uuid::new_v4().to_string();
                let label = m.source.label.clone().unwrap_or_else(|| kind.as_str().into());
                conn.execute(
                    "INSERT INTO memory_sources (id, kind, reference, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![source_id, kind.as_str(), m.source.reference, label, now()],
                )
                .map_err(err)?;
                conn.execute(
                    &format!("INSERT INTO memories ({MEMORY_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[]', ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
                    params![
                        id,
                        m.kind,
                        m.content,
                        m.created_at.to_rfc3339(),
                        label,
                        m.relevance,
                        m.confidence,
                        retention_json,
                        m.summary,
                        m.updated_at.to_rfc3339(),
                        m.verification,
                        tags,
                        source_id,
                        content_hash(&m.content)
                    ],
                )
                .map_err(err)?;
            }
            applied.upserted_memories.push(id.to_owned());
        }
        entity::RELATION => {
            let r: RelationData = serde_json::from_value(data.clone()).map_err(err)?;
            let endpoints: i64 = conn
                .query_row("SELECT COUNT(*) FROM memories WHERE id IN (?1, ?2)", params![r.source_id, r.target_id], |row| row.get(0))
                .map_err(err)?;
            if endpoints < 2 {
                return Ok(false);
            }
            let verified = r.verified && r.provenance != "inferred";
            conn.execute(
                "INSERT INTO memory_relations (id, source_id, target_id, kind, confidence, provenance, verified, note, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (id) DO UPDATE SET kind = excluded.kind, confidence = excluded.confidence,
                   provenance = excluded.provenance, verified = excluded.verified, note = excluded.note
                 ON CONFLICT (source_id, target_id, kind) DO NOTHING",
                params![
                    id,
                    r.source_id,
                    r.target_id,
                    r.kind,
                    r.confidence,
                    r.provenance,
                    i64::from(verified),
                    r.note,
                    r.created_at.unwrap_or_else(Utc::now).to_rfc3339()
                ],
            )
            .map_err(err)?;
            applied.relations += 1;
        }
        entity::EXPERIENCE => {
            let e: ExperienceData = serde_json::from_value(data.clone()).map_err(err)?;
            let promoted: bool = conn
                .query_row("SELECT promoted FROM experiences WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
                .optional()
                .map_err(err)?
                .is_some_and(|p| p != 0);
            let experience = Experience {
                id: id.to_owned(),
                goal: e.goal,
                context: e.context,
                plan: e.plan,
                steps: e
                    .steps
                    .into_iter()
                    .map(|s| ExperienceStep {
                        tool: s.tool,
                        arguments: s.arguments,
                        ok: s.ok,
                        verification: s
                            .verification
                            .and_then(|v| serde_json::from_value(Value::String(v.to_lowercase())).ok())
                            .unwrap_or(VerificationStatus::NotApplicable),
                    })
                    .collect(),
                result: e.result,
                evidence: e.evidence,
                errors: e.errors,
                procedure: e.procedure,
                verified: e.verified,
                promoted,
                created_at: e.created_at.unwrap_or_else(Utc::now),
                completed: e.completed,
                occurrences: e.occurrences,
            };
            conn.execute(
                "INSERT INTO experiences (id, data, verified, promoted, created_at, signature, occurrences, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET data = excluded.data, verified = excluded.verified,
                   signature = excluded.signature, occurrences = excluded.occurrences, updated_at = excluded.updated_at",
                params![
                    id,
                    serde_json::to_string(&experience).map_err(err)?,
                    i64::from(experience.verified),
                    i64::from(promoted),
                    experience.created_at.to_rfc3339(),
                    e.signature,
                    experience.occurrences,
                    now()
                ],
            )
            .map_err(err)?;
            applied.experiences += 1;
        }
        entity::SKILL => {
            let s: SkillData = serde_json::from_value(data.clone()).map_err(err)?;
            let compat = serde_json::to_string(&s.compatibility.unwrap_or_else(|| SkillCompatibility {
                platforms: vec!["windows".into()],
                required_tools: Vec::new(),
            }))
            .map_err(err)?;
            let perms = serde_json::to_string(&s.permissions.unwrap_or(SkillPermissions { max_risk: Some("R1".into()) })).map_err(err)?;
            let existing: Option<(String, i64)> = conn
                .query_row("SELECT procedure, current_version FROM skills WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .map_err(err)?;
            match existing {
                None => {
                    // Habilidad recibida: llega desactivada y pendiente de revisión local. Nunca se ejecuta sola.
                    conn.execute(
                        "INSERT INTO skills (id, name, description, procedure, enabled, requires_review, source, experience_id,
                           current_version, created_at, updated_at, status, compatibility, permissions)
                         VALUES (?1, ?2, ?3, ?4, 0, 1, 'cloud', ?5, 1, ?6, ?6, ?7, ?8, ?9)",
                        params![id, s.name, s.description, s.procedure, s.experience_id, now(), s.status, compat, perms],
                    )
                    .map_err(err)?;
                    conn.execute(
                        "INSERT INTO skill_versions (skill_id, version, procedure, notes, created_at) VALUES (?1, 1, ?2, 'recibida del Cloud Core', ?3)",
                        params![id, s.procedure, now()],
                    )
                    .map_err(err)?;
                }
                Some((procedure, version)) => {
                    let changed = procedure != s.procedure;
                    conn.execute(
                        "UPDATE skills SET name = ?2, description = ?3, procedure = ?4, status = ?5, compatibility = ?6, permissions = ?7,
                           updated_at = ?8, current_version = ?9,
                           enabled = CASE WHEN ?10 THEN 0 ELSE enabled END,
                           requires_review = CASE WHEN ?10 THEN 1 ELSE requires_review END
                         WHERE id = ?1",
                        params![id, s.name, s.description, s.procedure, s.status, compat, perms, now(), version + i64::from(changed), changed],
                    )
                    .map_err(err)?;
                    if changed {
                        conn.execute(
                            "INSERT OR IGNORE INTO skill_versions (skill_id, version, procedure, notes, created_at) VALUES (?1, ?2, ?3, 'actualizada desde el Cloud Core', ?4)",
                            params![id, version + 1, s.procedure, now()],
                        )
                        .map_err(err)?;
                    }
                }
            }
            applied.skills += 1;
        }
        _ => {}
    }
    Ok(true)
}

fn apply_delete(conn: &Connection, entity: &str, id: &str, applied: &mut Applied) -> StoreResult<()> {
    let table = match entity {
        entity::MEMORY => "memories",
        entity::RELATION => "memory_relations",
        entity::EXPERIENCE => "experiences",
        entity::SKILL => "skills",
        _ => return Ok(()),
    };
    let n = conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id]).map_err(err)?;
    if n > 0 && entity == entity::MEMORY {
        applied.deleted_memories.push(id.to_owned());
    }
    Ok(())
}

/// Un vector recibido se guarda solo si es del modelo local y del contenido local exacto.
fn apply_embedding(conn: &Connection, memory_id: &str, local_model: &str, data: &Value, applied: &mut Applied) -> StoreResult<()> {
    let e: EmbeddingData = serde_json::from_value(data.clone()).map_err(err)?;
    if e.model != local_model || e.vector.len() != e.dims {
        return Ok(());
    }
    let Some(content): Option<String> =
        conn.query_row("SELECT content FROM memories WHERE id = ?1", [memory_id], |r| r.get(0)).optional().map_err(err)?
    else {
        return Ok(());
    };
    if content_sha256(&content) != e.content_hash {
        return Ok(());
    }
    let blob: Vec<u8> = e.vector.iter().flat_map(|x| x.to_le_bytes()).collect();
    conn.execute(
        "INSERT INTO embedding_metadata (memory_id, model, dims, content_hash, vector, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (memory_id, model) DO UPDATE SET dims = excluded.dims, content_hash = excluded.content_hash, vector = excluded.vector",
        params![memory_id, e.model, i64::try_from(e.dims).unwrap_or(0), content_hash(&content), blob, now()],
    )
    .map_err(err)?;
    applied.embeddings += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::memory::{MemoryKind, NewMemory};
    use crate::memory::MemoryEngine;
    use crate::sync::protocol::PushResult;

    fn engine() -> (MemoryEngine, SyncStore, ()) {
        let engine = MemoryEngine::for_tests().expect("engine");
        let store = SyncStore::new(engine.database().clone());
        (engine, store, ())
    }

    fn result(change: &Change, status: &str, version: i64) -> PushResult {
        PushResult {
            change_id: change.change_id.clone(),
            status: status.into(),
            entity: Some(change.entity.clone()),
            entity_id: Some(change.entity_id.clone()),
            version: Some(version),
            seq: Some(1),
            data: None,
            conflict_id: None,
            fields: None,
            conflict_kind: None,
            code: None,
            message: None,
            replayed: false,
        }
    }

    #[tokio::test]
    async fn triggers_capture_changes_only_when_enabled_and_not_while_applying_remote_changes() {
        let (engine, store, _dir) = engine();
        engine.remember(NewMemory::user(MemoryKind::Semantic, "Antes de emparejar")).await.expect("m");
        assert_eq!(store.outbox_stats().expect("s").pending, 0, "sin sincronización no se acumula nada");

        let enqueued = store.enable_and_enqueue_all().expect("enable");
        assert!(enqueued >= 1);
        let first = engine.remember(NewMemory::user(MemoryKind::Semantic, "Después de emparejar")).await.expect("m").memory;
        engine.update_memory(&first.id, crate::domain::MemoryPatch { content: Some("Editado".into()), ..Default::default() }).await.expect("u");
        let batch = store.next_batch(200, 1 << 20).expect("batch");
        let mine: Vec<&Change> = batch.changes.iter().filter(|c| c.entity_id == first.id).collect();
        assert_eq!(mine.len(), 1, "dos ediciones locales viajan como un solo cambio con el estado actual");
        assert_eq!(mine[0].data.as_ref().expect("data")["content"], "Editado");
        assert_eq!(mine[0].base_version, None);

        // Un cambio recibido del Cloud no se vuelve a encolar.
        let pulled = PulledChange {
            seq: 10,
            entity: "memory".into(),
            entity_id: "0192f0a0-0000-7000-8000-00000000ab01".into(),
            op: "upsert".into(),
            version: 3,
            origin_device_id: None,
            change_id: None,
            data: Some(serde_json::json!({
                "kind": "semantic", "content": "Desde otra PC", "summary": "", "tags": ["remoto"], "relevance": 0.5,
                "confidence": 0.9, "verification": "verified", "retention": {"kind": "permanent"},
                "source": {"kind": "user", "label": "usuario"}, "createdAt": "2026-10-22T15:00:00Z", "updatedAt": "2026-10-22T15:00:00Z"
            })),
        };
        let mut applied = Applied::default();
        assert!(store.apply_pulled(&[pulled], None, &mut applied).expect("pull").is_empty());
        assert_eq!(applied.upserted_memories, vec!["0192f0a0-0000-7000-8000-00000000ab01".to_owned()]);
        let after = store.next_batch(200, 1 << 20).expect("batch");
        assert!(after.changes.iter().all(|c| c.entity_id != "0192f0a0-0000-7000-8000-00000000ab01"));
        assert_eq!(store.server_version("memory", "0192f0a0-0000-7000-8000-00000000ab01").expect("v"), Some(3));
        let record = engine.repository().get("0192f0a0-0000-7000-8000-00000000ab01").expect("g").expect("existe");
        assert_eq!(record.content, "Desde otra PC");
        assert_eq!(record.tags, vec!["remoto".to_owned()]);
    }

    #[tokio::test]
    async fn acknowledgements_set_versions_store_conflicts_and_keep_rejections() {
        let (engine, store, _dir) = engine();
        store.enable_and_enqueue_all().expect("enable");
        let a = engine.remember(NewMemory::user(MemoryKind::Semantic, "Uno")).await.expect("m").memory;
        let b = engine.remember(NewMemory::user(MemoryKind::Semantic, "Dos")).await.expect("m").memory;
        let c = engine.remember(NewMemory::user(MemoryKind::Semantic, "Tres")).await.expect("m").memory;
        let batch = store.next_batch(200, 1 << 20).expect("batch");
        let by_id = |id: &str| batch.changes.iter().find(|ch| ch.entity_id == id && ch.entity == "memory").expect("cambio").clone();
        let mut conflict = result(&by_id(&b.id), "conflict", 4);
        conflict.conflict_id = Some("conf-1".into());
        conflict.conflict_kind = Some("update_update".into());
        conflict.fields = Some(vec!["content".into()]);
        let mut rejected = result(&by_id(&c.id), "rejected", 0);
        rejected.code = Some("CONTENT_REJECTED".into());
        let mut results = vec![result(&by_id(&a.id), "applied", 1), conflict, rejected];
        results.extend(batch.changes.iter().filter(|ch| ![&a.id, &b.id, &c.id].contains(&&ch.entity_id)).map(|ch| result(ch, "applied", 1)));
        store.acknowledge(&batch, &results).expect("ack");

        assert_eq!(store.server_version("memory", &a.id).expect("v"), Some(1));
        assert_eq!(store.server_version("memory", &b.id).expect("v"), None, "un conflicto no adelanta la versión base");
        let conflicts = store.open_conflicts().expect("c");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].fields, vec!["content".to_owned()]);
        let stats = store.outbox_stats().expect("s");
        assert_eq!((stats.pending, stats.rejected), (0, 1));
        assert!(stats.last_error.expect("error").contains("CONTENT_REJECTED"));
        assert!(store.next_batch(200, 1 << 20).expect("batch").changes.is_empty(), "lo rechazado no se reintenta");
    }

    #[tokio::test]
    async fn pulled_changes_never_overwrite_pending_local_edits_and_skills_arrive_disabled() {
        let (engine, store, _dir) = engine();
        store.enable_and_enqueue_all().expect("enable");
        let local = engine.remember(NewMemory::user(MemoryKind::Semantic, "Edición local sin enviar")).await.expect("m").memory;
        let mut applied = Applied::default();
        let remote = PulledChange {
            seq: 5,
            entity: "memory".into(),
            entity_id: local.id.clone(),
            op: "upsert".into(),
            version: 2,
            origin_device_id: None,
            change_id: None,
            data: Some(serde_json::json!({
                "kind": "semantic", "content": "Versión remota", "retention": {"kind": "permanent"},
                "source": {"kind": "user"}, "createdAt": "2026-10-22T15:00:00Z", "updatedAt": "2026-10-22T15:00:00Z"
            })),
        };
        let skill = PulledChange {
            seq: 6,
            entity: "skill".into(),
            entity_id: "0192f0a0-0000-7000-8000-0000000005c1".into(),
            op: "upsert".into(),
            version: 1,
            origin_device_id: None,
            change_id: None,
            data: Some(serde_json::json!({ "name": "Abrir notas", "procedure": "1. open_app", "status": "verified" })),
        };
        store.apply_pulled(&[remote, skill], None, &mut applied).expect("pull");
        assert_eq!(applied.skipped_pending, 1);
        assert_eq!(engine.repository().get(&local.id).expect("g").expect("m").content, "Edición local sin enviar");
        let (enabled, review, status): (i64, i64, String) = engine
            .database()
            .conn()
            .query_row("SELECT enabled, requires_review, status FROM skills WHERE id = '0192f0a0-0000-7000-8000-0000000005c1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("skill");
        assert_eq!((enabled, review, status.as_str()), (0, 1, "verified"), "verificada en el registro, pero desactivada aquí");
    }

    #[tokio::test]
    async fn relations_whose_endpoints_arrive_later_are_deferred() {
        let (_engine, store, _dir) = engine();
        let memory = |id: &str, seq: i64| PulledChange {
            seq,
            entity: "memory".into(),
            entity_id: id.into(),
            op: "upsert".into(),
            version: 2,
            origin_device_id: None,
            change_id: None,
            data: Some(serde_json::json!({
                "kind": "entity", "content": format!("Recuerdo {id}"), "retention": {"kind": "permanent"},
                "source": {"kind": "user"}, "createdAt": "2026-10-22T15:00:00Z", "updatedAt": "2026-10-22T15:00:00Z"
            })),
        };
        let relation = PulledChange {
            seq: 3,
            entity: "relation".into(),
            entity_id: "0192f0a0-0000-7000-8000-0000000000aa".into(),
            op: "upsert".into(),
            version: 1,
            origin_device_id: None,
            change_id: None,
            data: Some(serde_json::json!({ "sourceId": "m-1", "targetId": "m-2", "kind": "DUPLICATE_OF", "provenance": "consolidation" })),
        };
        let mut applied = Applied::default();
        let deferred = store.apply_pulled(&[relation, memory("m-1", 7)], None, &mut applied).expect("pull");
        assert_eq!(deferred.len(), 1);
        store.apply_pulled(&[memory("m-2", 8)], None, &mut applied).expect("pull");
        assert!(store.apply_pulled(&deferred, None, &mut applied).expect("retry").is_empty());
        assert_eq!(applied.relations, 1);
    }
}
