//! Acceso SQLite a recuerdos, fuentes, relaciones, embeddings y eventos.
//! Solo SQL: las reglas (secretos, verificación, eventos visuales) están en `MemoryEngine`.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{MemoryError, MemoryResult};
use crate::domain::{
    MemoryKind, MemoryRecord, MemoryRelation, MemorySource, NewSource, Provenance, RelationKind, RetentionPolicy,
    SourceKind, Verification,
};
use crate::infrastructure::persistence::Database;

pub(crate) fn storage<E: std::fmt::Display>(e: E) -> MemoryError {
    MemoryError::Storage(e.to_string())
}

/// Hash del contenido normalizado (minúsculas, espacios colapsados): detecta duplicados.
#[must_use]
pub fn content_hash(content: &str) -> String {
    let normalized = content.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    hex(&Sha256::digest(normalized.as_bytes()))
}

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_time(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc))
}

pub(crate) const MEMORY_COLUMNS: &str = "id, kind, content, created_at, source, relevance, confidence, refs, retention, \
     summary, updated_at, verification, tags, source_id, content_hash";

pub(crate) fn row_to_memory(r: &Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let kind: String = r.get(1)?;
    let created: String = r.get(3)?;
    let refs: String = r.get(7)?;
    let retention: String = r.get(8)?;
    let updated: String = r.get(10)?;
    let verification: String = r.get(11)?;
    let tags: String = r.get(12)?;
    let created_at = parse_time(&created);
    Ok(MemoryRecord {
        id: r.get(0)?,
        kind: MemoryKind::parse(&kind).unwrap_or(MemoryKind::Semantic),
        content: r.get(2)?,
        created_at,
        source: r.get(4)?,
        relevance: r.get(5)?,
        confidence: r.get(6)?,
        references: serde_json::from_str(&refs).unwrap_or_default(),
        retention: serde_json::from_str(&retention).unwrap_or(RetentionPolicy::Permanent),
        summary: r.get(9)?,
        updated_at: if updated.is_empty() { created_at } else { parse_time(&updated) },
        verification: Verification::parse(&verification).unwrap_or_default(),
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        source_id: r.get(13)?,
        content_hash: r.get(14)?,
    })
}

const RELATION_COLUMNS: &str = "id, source_id, target_id, kind, confidence, provenance, verified, note, created_at";

fn row_to_relation(r: &Row<'_>) -> rusqlite::Result<MemoryRelation> {
    let kind: String = r.get(3)?;
    let provenance: String = r.get(5)?;
    let created: String = r.get(8)?;
    Ok(MemoryRelation {
        id: r.get(0)?,
        source_id: r.get(1)?,
        target_id: r.get(2)?,
        kind: RelationKind::parse(&kind).unwrap_or(RelationKind::RelatedTo),
        confidence: r.get(4)?,
        provenance: Provenance::parse(&provenance).unwrap_or(Provenance::Inferred),
        verified: r.get::<_, i64>(6)? != 0,
        note: r.get(7)?,
        created_at: parse_time(&created),
    })
}

/// Filtro de listados (pantalla de memoria).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFilter {
    pub kind: Option<MemoryKind>,
    pub verification: Option<Verification>,
    pub query: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPage {
    pub items: Vec<MemoryRecord>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEventRow {
    pub id: i64,
    pub ts: String,
    pub kind: String,
    pub run_id: Option<String>,
    pub memory_ids: Vec<String>,
    pub detail: Option<String>,
}

#[derive(Clone)]
pub struct MemoryRepository {
    db: Database,
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

impl MemoryRepository {
    /// # Errors
    /// Si falla el relleno de hashes de recuerdos antiguos.
    pub fn new(db: Database) -> MemoryResult<Self> {
        let repo = Self { db };
        repo.backfill_hashes()?;
        Ok(repo)
    }

    #[must_use]
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Recuerdos del esquema v1 no tenían hash.
    fn backfill_hashes(&self) -> MemoryResult<()> {
        let conn = self.db.conn();
        let pending: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT id, content FROM memories WHERE content_hash = ''").map_err(storage)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(storage)?;
            rows.collect::<Result<_, _>>().map_err(storage)?
        };
        for (id, content) in pending {
            conn.execute("UPDATE memories SET content_hash = ?2 WHERE id = ?1", params![id, content_hash(&content)])
                .map_err(storage)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ fuentes

    /// # Errors
    /// Errores de SQLite.
    pub fn insert_source(&self, source: &NewSource) -> MemoryResult<MemorySource> {
        let s = MemorySource {
            id: uuid::Uuid::new_v4().to_string(),
            kind: source.kind,
            reference: source.reference.clone(),
            label: source.label.clone().unwrap_or_else(|| source.kind.as_str().to_owned()),
            created_at: Utc::now(),
        };
        self.db
            .conn()
            .execute(
                "INSERT INTO memory_sources (id, kind, reference, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![s.id, s.kind.as_str(), s.reference, s.label, s.created_at.to_rfc3339()],
            )
            .map_err(storage)?;
        Ok(s)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn get_source(&self, id: &str) -> MemoryResult<Option<MemorySource>> {
        self.db
            .conn()
            .query_row("SELECT id, kind, reference, label, created_at FROM memory_sources WHERE id = ?1", [id], |r| {
                let kind: String = r.get(1)?;
                let created: String = r.get(4)?;
                Ok(MemorySource {
                    id: r.get(0)?,
                    kind: SourceKind::parse(&kind).unwrap_or(SourceKind::User),
                    reference: r.get(2)?,
                    label: r.get(3)?,
                    created_at: parse_time(&created),
                })
            })
            .optional()
            .map_err(storage)
    }

    // ------------------------------------------------------------------ recuerdos

    /// # Errors
    /// Errores de SQLite (incluido un id repetido).
    pub fn insert(&self, m: &MemoryRecord) -> MemoryResult<()> {
        self.db
            .conn()
            .execute(
                &format!("INSERT INTO memories ({MEMORY_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"),
                params![
                    m.id,
                    m.kind.as_str(),
                    m.content,
                    m.created_at.to_rfc3339(),
                    m.source,
                    m.relevance,
                    m.confidence,
                    serde_json::to_string(&m.references).map_err(storage)?,
                    serde_json::to_string(&m.retention).map_err(storage)?,
                    m.summary,
                    m.updated_at.to_rfc3339(),
                    m.verification.as_str(),
                    serde_json::to_string(&m.tags).map_err(storage)?,
                    m.source_id,
                    m.content_hash,
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn update(&self, m: &MemoryRecord) -> MemoryResult<()> {
        let n = self
            .db
            .conn()
            .execute(
                "UPDATE memories SET content = ?2, summary = ?3, tags = ?4, confidence = ?5, verification = ?6,
                 retention = ?7, updated_at = ?8, content_hash = ?9 WHERE id = ?1",
                params![
                    m.id,
                    m.content,
                    m.summary,
                    serde_json::to_string(&m.tags).map_err(storage)?,
                    m.confidence,
                    m.verification.as_str(),
                    serde_json::to_string(&m.retention).map_err(storage)?,
                    m.updated_at.to_rfc3339(),
                    m.content_hash,
                ],
            )
            .map_err(storage)?;
        if n == 0 {
            Err(MemoryError::NotFound)
        } else {
            Ok(())
        }
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn get(&self, id: &str) -> MemoryResult<Option<MemoryRecord>> {
        self.db
            .conn()
            .query_row(&format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = ?1"), [id], row_to_memory)
            .optional()
            .map_err(storage)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn get_many(&self, ids: &[String]) -> MemoryResult<Vec<MemoryRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.db.conn();
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut stmt = conn
            .prepare(&format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id IN ({placeholders})"))
            .map_err(storage)?;
        let rows = stmt.query_map(params_from_iter(ids.iter()), row_to_memory).map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// Id completo a partir de un prefijo (los ids cortos `[m:xxxxxxxx]` del contexto).
    /// Devuelve None si no existe o es ambiguo.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn resolve_prefix(&self, prefix: &str) -> MemoryResult<Option<String>> {
        let prefix: String = prefix.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        if prefix.len() < 6 {
            return Ok(None);
        }
        let conn = self.db.conn();
        let mut stmt = conn.prepare("SELECT id FROM memories WHERE id LIKE ?1 LIMIT 2").map_err(storage)?;
        let ids: Vec<String> =
            stmt.query_map([format!("{prefix}%")], |r| r.get(0)).map_err(storage)?.collect::<Result<_, _>>().map_err(storage)?;
        Ok(if ids.len() == 1 { ids.into_iter().next() } else { None })
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn find_by_hash(&self, hash: &str) -> MemoryResult<Option<MemoryRecord>> {
        self.db
            .conn()
            .query_row(&format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE content_hash = ?1 LIMIT 1"), [hash], row_to_memory)
            .optional()
            .map_err(storage)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn delete(&self, id: &str) -> MemoryResult<bool> {
        Ok(self.db.conn().execute("DELETE FROM memories WHERE id = ?1", [id]).map_err(storage)? > 0)
    }

    /// Listado paginado.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn list(&self, filter: &MemoryFilter, offset: u64, limit: u64) -> MemoryResult<MemoryPage> {
        let conn = self.db.conn();
        let like = filter.query.as_ref().filter(|q| !q.trim().is_empty()).map(|q| format!("%{}%", q.trim()));
        let tag = filter.tag.as_ref().filter(|t| !t.trim().is_empty()).map(|t| format!("%\"{}\"%", t.trim()));
        let kind = filter.kind.map(MemoryKind::as_str);
        let verification = filter.verification.map(Verification::as_str);
        let condition = "(?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR verification = ?2)
             AND (?3 IS NULL OR content LIKE ?3 OR summary LIKE ?3) AND (?4 IS NULL OR tags LIKE ?4)";
        let total: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM memories WHERE {condition}"), params![kind, verification, like, tag], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE {condition} ORDER BY updated_at DESC, id LIMIT ?5 OFFSET ?6"
            ))
            .map_err(storage)?;
        let limit = limit.clamp(1, 500);
        let rows = stmt
            .query_map(
                params![kind, verification, like, tag, i64::try_from(limit).unwrap_or(500), i64::try_from(offset).unwrap_or(0)],
                row_to_memory,
            )
            .map_err(storage)?;
        Ok(MemoryPage {
            items: rows.collect::<Result<_, _>>().map_err(storage)?,
            total: u64::try_from(total).unwrap_or(0),
            offset,
            limit,
        })
    }

    /// Los más recientes (para la vista general del grafo).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn recent(&self, limit: usize, offset: usize) -> MemoryResult<Vec<MemoryRecord>> {
        let conn = self.db.conn();
        let mut stmt = conn
            .prepare(&format!("SELECT {MEMORY_COLUMNS} FROM memories ORDER BY updated_at DESC, id LIMIT ?1 OFFSET ?2"))
            .map_err(storage)?;
        let rows = stmt
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX), i64::try_from(offset).unwrap_or(0)], row_to_memory)
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn count(&self) -> MemoryResult<u64> {
        let n: i64 = self.db.conn().query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0)).map_err(storage)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn counts_by_kind(&self) -> MemoryResult<HashMap<String, u64>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM memories GROUP BY kind").map_err(storage)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0))))
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    // ------------------------------------------------------------------ relaciones

    /// Inserta una relación. Devuelve `false` si ya existía la misma (origen, destino, tipo).
    ///
    /// # Errors
    /// Errores de SQLite (por ejemplo, un extremo inexistente).
    pub fn insert_relation(&self, r: &MemoryRelation) -> MemoryResult<bool> {
        let n = self
            .db
            .conn()
            .execute(
                &format!("INSERT OR IGNORE INTO memory_relations ({RELATION_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
                params![
                    r.id,
                    r.source_id,
                    r.target_id,
                    r.kind.as_str(),
                    r.confidence,
                    r.provenance.as_str(),
                    i64::from(r.verified),
                    r.note,
                    r.created_at.to_rfc3339(),
                ],
            )
            .map_err(storage)?;
        Ok(n > 0)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn get_relation(&self, id: &str) -> MemoryResult<Option<MemoryRelation>> {
        self.db
            .conn()
            .query_row(&format!("SELECT {RELATION_COLUMNS} FROM memory_relations WHERE id = ?1"), [id], row_to_relation)
            .optional()
            .map_err(storage)
    }

    /// Relaciones que tocan a un recuerdo (en cualquier dirección).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn relations_of(&self, id: &str) -> MemoryResult<Vec<MemoryRelation>> {
        let conn = self.db.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {RELATION_COLUMNS} FROM memory_relations WHERE source_id = ?1 OR target_id = ?1 ORDER BY confidence DESC"
            ))
            .map_err(storage)?;
        let rows = stmt.query_map([id], row_to_relation).map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// Relaciones con ambos extremos dentro del conjunto.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn relations_among(&self, ids: &HashSet<String>) -> MemoryResult<Vec<MemoryRelation>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let list: Vec<&String> = ids.iter().collect();
        let placeholders = vec!["?"; list.len()].join(",");
        let conn = self.db.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {RELATION_COLUMNS} FROM memory_relations WHERE source_id IN ({placeholders}) AND target_id IN ({placeholders})"
            ))
            .map_err(storage)?;
        let params: Vec<&String> = list.iter().chain(list.iter()).copied().collect();
        let rows = stmt.query_map(params_from_iter(params), row_to_relation).map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn all_relations(&self) -> MemoryResult<Vec<MemoryRelation>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(&format!("SELECT {RELATION_COLUMNS} FROM memory_relations")).map_err(storage)?;
        let rows = stmt.query_map([], row_to_relation).map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn set_relation_verified(&self, id: &str, verified: bool) -> MemoryResult<bool> {
        Ok(self
            .db
            .conn()
            .execute("UPDATE memory_relations SET verified = ?2 WHERE id = ?1", params![id, i64::from(verified)])
            .map_err(storage)?
            > 0)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn delete_relation(&self, id: &str) -> MemoryResult<bool> {
        Ok(self.db.conn().execute("DELETE FROM memory_relations WHERE id = ?1", [id]).map_err(storage)? > 0)
    }

    /// Grado (número de relaciones) de cada recuerdo pedido.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn degrees(&self, ids: &[String]) -> MemoryResult<HashMap<String, u32>> {
        let mut out: HashMap<String, u32> = ids.iter().map(|i| (i.clone(), 0)).collect();
        if ids.is_empty() {
            return Ok(out);
        }
        let conn = self.db.conn();
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, COUNT(*) FROM (
                    SELECT source_id AS id FROM memory_relations WHERE source_id IN ({placeholders})
                    UNION ALL
                    SELECT target_id AS id FROM memory_relations WHERE target_id IN ({placeholders})
                 ) GROUP BY id"
            ))
            .map_err(storage)?;
        let params: Vec<&String> = ids.iter().chain(ids.iter()).collect();
        let rows = stmt
            .query_map(params_from_iter(params), |r| Ok((r.get::<_, String>(0)?, u32::try_from(r.get::<_, i64>(1)?).unwrap_or(0))))
            .map_err(storage)?;
        for row in rows {
            let (id, n) = row.map_err(storage)?;
            out.insert(id, n);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------ embeddings

    /// # Errors
    /// Errores de SQLite.
    pub fn upsert_embedding(&self, memory_id: &str, model: &str, hash: &str, vector: &[f32]) -> MemoryResult<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO embedding_metadata (memory_id, model, dims, content_hash, vector, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (memory_id, model) DO UPDATE SET dims = excluded.dims, content_hash = excluded.content_hash,
                   vector = excluded.vector, created_at = excluded.created_at",
                params![memory_id, model, i64::try_from(vector.len()).unwrap_or(0), hash, vec_to_blob(vector), Utc::now().to_rfc3339()],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Embeddings vigentes del modelo (los de otros modelos se ignoran).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn embeddings_for_model(&self, model: &str) -> MemoryResult<Vec<(String, String, Vec<f32>)>> {
        let conn = self.db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT e.memory_id, e.content_hash, e.vector FROM embedding_metadata e
                 JOIN memories m ON m.id = e.memory_id AND m.content_hash = e.content_hash
                 WHERE e.model = ?1",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map([model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, blob_to_vec(&r.get::<_, Vec<u8>>(2)?))))
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// Recuerdos sin embedding vigente para el modelo.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn missing_embeddings(&self, model: &str, limit: usize) -> MemoryResult<Vec<MemoryRecord>> {
        let conn = self.db.conn();
        let cols = MEMORY_COLUMNS.split(", ").map(|c| format!("m.{c}")).collect::<Vec<_>>().join(", ");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {cols} FROM memories m
                 LEFT JOIN embedding_metadata e ON e.memory_id = m.id AND e.model = ?1 AND e.content_hash = m.content_hash
                 WHERE e.memory_id IS NULL LIMIT ?2"
            ))
            .map_err(storage)?;
        let rows = stmt.query_map(params![model, i64::try_from(limit).unwrap_or(i64::MAX)], row_to_memory).map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn count_missing_embeddings(&self, model: &str) -> MemoryResult<u64> {
        let n: i64 = self
            .db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM memories m
                 LEFT JOIN embedding_metadata e ON e.memory_id = m.id AND e.model = ?1 AND e.content_hash = m.content_hash
                 WHERE e.memory_id IS NULL",
                [model],
                |r| r.get(0),
            )
            .map_err(storage)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    // ------------------------------------------------------------------ eventos

    /// # Errors
    /// Errores de SQLite.
    pub fn log_event(&self, kind: &str, run_id: Option<&str>, memory_ids: &[String], detail: Option<&str>) -> MemoryResult<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO memory_events (ts, kind, run_id, memory_ids, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![Utc::now().to_rfc3339(), kind, run_id, serde_json::to_string(memory_ids).map_err(storage)?, detail],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Eventos que mencionan un recuerdo (o todos si `memory_id` es None).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn events(&self, memory_id: Option<&str>, run_id: Option<&str>, limit: usize) -> MemoryResult<Vec<MemoryEventRow>> {
        let conn = self.db.conn();
        let like = memory_id.map(|id| format!("%\"{id}\"%"));
        let mut stmt = conn
            .prepare(
                "SELECT id, ts, kind, run_id, memory_ids, detail FROM memory_events
                 WHERE (?1 IS NULL OR memory_ids LIKE ?1) AND (?2 IS NULL OR run_id = ?2)
                 ORDER BY id DESC LIMIT ?3",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map(params![like, run_id, i64::try_from(limit).unwrap_or(100)], |r| {
                let ids: String = r.get(4)?;
                Ok(MemoryEventRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    kind: r.get(2)?,
                    run_id: r.get(3)?,
                    memory_ids: serde_json::from_str(&ids).unwrap_or_default(),
                    detail: r.get(5)?,
                })
            })
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    // ------------------------------------------------------------------ metadatos

    /// Identificador estable de esta instalación (se crea la primera vez).
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn installation_id(&self) -> MemoryResult<String> {
        let conn = self.db.conn();
        if let Some(id) = conn
            .query_row("SELECT value FROM app_meta WHERE key = 'installation_id'", [], |r| r.get::<_, String>(0))
            .optional()
            .map_err(storage)?
        {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute("INSERT INTO app_meta (key, value) VALUES ('installation_id', ?1)", [&id]).map_err(storage)?;
        Ok(id)
    }
}
