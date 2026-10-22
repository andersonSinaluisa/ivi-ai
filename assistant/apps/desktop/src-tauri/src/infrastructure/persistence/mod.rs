//! SQLite local. Un solo archivo (`%LOCALAPPDATA%\Jarvis\jarvis.db`), migraciones en orden.

pub mod audit;
pub mod conversations;
pub mod runs;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type DbResult<T> = Result<T, DbError>;

const MIGRATIONS: &[&str] = &[
    // 1: esquema inicial
    r"
    CREATE TABLE conversations (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE messages (
        id TEXT PRIMARY KEY,
        conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
        role TEXT NOT NULL,
        content TEXT NOT NULL,
        tool_calls TEXT,
        tool_name TEXT,
        created_at TEXT NOT NULL
    );
    CREATE INDEX idx_messages_conversation ON messages(conversation_id, created_at);

    CREATE TABLE memories (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        content TEXT NOT NULL,
        created_at TEXT NOT NULL,
        source TEXT NOT NULL,
        relevance REAL NOT NULL,
        confidence REAL NOT NULL,
        refs TEXT NOT NULL,
        retention TEXT NOT NULL
    );
    CREATE INDEX idx_memories_kind ON memories(kind);

    CREATE TABLE experiences (
        id TEXT PRIMARY KEY,
        data TEXT NOT NULL,
        verified INTEGER NOT NULL,
        promoted INTEGER NOT NULL,
        created_at TEXT NOT NULL
    );

    CREATE TABLE runs (
        id TEXT PRIMARY KEY,
        conversation_id TEXT NOT NULL,
        goal TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at TEXT NOT NULL,
        finished_at TEXT,
        error TEXT,
        tool_calls INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE audit_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts TEXT NOT NULL,
        run_id TEXT,
        actor TEXT NOT NULL,
        action TEXT NOT NULL,
        tool TEXT,
        risk TEXT,
        decision TEXT,
        outcome TEXT,
        detail TEXT
    );
    CREATE INDEX idx_audit_ts ON audit_log(ts);
    ",
    // 2: Memory Engine — recuerdos ampliados, grafo, fuentes, habilidades, eventos y embeddings.
    r"
    ALTER TABLE memories ADD COLUMN summary TEXT NOT NULL DEFAULT '';
    ALTER TABLE memories ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
    ALTER TABLE memories ADD COLUMN verification TEXT NOT NULL DEFAULT 'unverified';
    ALTER TABLE memories ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE memories ADD COLUMN source_id TEXT;
    ALTER TABLE memories ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
    UPDATE memories SET updated_at = created_at WHERE updated_at = '';
    UPDATE memories SET verification = 'verified' WHERE source = 'user';
    CREATE INDEX idx_memories_updated ON memories(updated_at);
    CREATE INDEX idx_memories_hash ON memories(content_hash);
    CREATE INDEX idx_memories_verification ON memories(verification);

    CREATE TABLE memory_sources (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        reference TEXT,
        label TEXT NOT NULL,
        created_at TEXT NOT NULL
    );

    CREATE TABLE memory_relations (
        id TEXT PRIMARY KEY,
        source_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        target_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        confidence REAL NOT NULL,
        provenance TEXT NOT NULL,
        verified INTEGER NOT NULL DEFAULT 0,
        note TEXT,
        created_at TEXT NOT NULL,
        UNIQUE (source_id, target_id, kind),
        CHECK (source_id <> target_id)
    );
    CREATE INDEX idx_relations_source ON memory_relations(source_id);
    CREATE INDEX idx_relations_target ON memory_relations(target_id);

    ALTER TABLE experiences ADD COLUMN signature TEXT NOT NULL DEFAULT '';
    ALTER TABLE experiences ADD COLUMN occurrences INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE experiences ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
    CREATE INDEX idx_experiences_signature ON experiences(signature);

    CREATE TABLE skills (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT NOT NULL,
        procedure TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        requires_review INTEGER NOT NULL DEFAULT 0,
        source TEXT NOT NULL,
        experience_id TEXT,
        current_version INTEGER NOT NULL DEFAULT 1,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE skill_versions (
        skill_id TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
        version INTEGER NOT NULL,
        procedure TEXT NOT NULL,
        notes TEXT,
        created_at TEXT NOT NULL,
        PRIMARY KEY (skill_id, version)
    );

    CREATE TABLE memory_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts TEXT NOT NULL,
        kind TEXT NOT NULL,
        run_id TEXT,
        memory_ids TEXT NOT NULL DEFAULT '[]',
        detail TEXT
    );
    CREATE INDEX idx_memory_events_ts ON memory_events(ts);
    CREATE INDEX idx_memory_events_run ON memory_events(run_id);

    CREATE TABLE embedding_metadata (
        memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        model TEXT NOT NULL,
        dims INTEGER NOT NULL,
        content_hash TEXT NOT NULL,
        vector BLOB NOT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY (memory_id, model)
    );
    CREATE INDEX idx_embeddings_model ON embedding_metadata(model);

    CREATE TABLE app_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    ",
    // 3: sincronización con el Cloud Core (ADR-015): outbox capturada por triggers en la misma transacción que
    //    el cambio, estado por registro, conflictos locales y metadatos de habilidades del registro.
    r#"
    CREATE TABLE sync_outbox (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        change_id TEXT NOT NULL UNIQUE,
        entity TEXT NOT NULL,
        entity_id TEXT NOT NULL,
        op TEXT NOT NULL,
        detail TEXT,
        created_at TEXT NOT NULL,
        attempts INTEGER NOT NULL DEFAULT 0,
        status TEXT NOT NULL DEFAULT 'pending',
        last_error TEXT
    );
    CREATE INDEX idx_sync_outbox_entity ON sync_outbox(entity, entity_id);
    CREATE INDEX idx_sync_outbox_status ON sync_outbox(status, seq);

    CREATE TABLE sync_state (
        entity TEXT NOT NULL,
        entity_id TEXT NOT NULL,
        server_version INTEGER NOT NULL,
        conflict_id TEXT,
        synced_at TEXT NOT NULL,
        PRIMARY KEY (entity, entity_id)
    );

    CREATE TABLE sync_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE sync_conflicts (
        id TEXT PRIMARY KEY,
        entity TEXT NOT NULL,
        entity_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        fields TEXT NOT NULL DEFAULT '[]',
        server_version INTEGER NOT NULL,
        server_data TEXT,
        device_data TEXT,
        status TEXT NOT NULL DEFAULT 'open',
        created_at TEXT NOT NULL
    );

    ALTER TABLE skills ADD COLUMN status TEXT NOT NULL DEFAULT 'draft';
    ALTER TABLE skills ADD COLUMN compatibility TEXT NOT NULL DEFAULT '{"platforms":["windows"],"requiredTools":[]}';
    ALTER TABLE skills ADD COLUMN permissions TEXT NOT NULL DEFAULT '{"maxRisk":"R1"}';

    CREATE TRIGGER trg_sync_memories_insert AFTER INSERT ON memories
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress') AND NEW.retention NOT LIKE '%session%'
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'memory', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_memories_update AFTER UPDATE OF kind, content, summary, tags, confidence, verification, retention ON memories
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress') AND NEW.retention NOT LIKE '%session%'
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'memory', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_memories_delete AFTER DELETE ON memories
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress') AND OLD.retention NOT LIKE '%session%'
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'memory', OLD.id, 'delete', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_relations_insert AFTER INSERT ON memory_relations
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'relation', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_relations_update AFTER UPDATE OF kind, confidence, provenance, verified, note ON memory_relations
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'relation', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_relations_delete AFTER DELETE ON memory_relations
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'relation', OLD.id, 'delete', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_experiences_insert AFTER INSERT ON experiences
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'experience', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_experiences_update AFTER UPDATE OF data, verified, occurrences ON experiences
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'experience', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_experiences_delete AFTER DELETE ON experiences
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'experience', OLD.id, 'delete', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_skills_insert AFTER INSERT ON skills
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'skill', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_skills_update AFTER UPDATE OF name, description, procedure ON skills
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'skill', NEW.id, 'upsert', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_skills_delete AFTER DELETE ON skills
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress')
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'skill', OLD.id, 'delete', NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_embeddings_insert AFTER INSERT ON embedding_metadata
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress') AND NEW.model NOT LIKE 'lexical-%'
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'embedding', NEW.memory_id, 'upsert', NEW.model, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    CREATE TRIGGER trg_sync_embeddings_update AFTER UPDATE ON embedding_metadata
    WHEN EXISTS (SELECT 1 FROM sync_meta WHERE key = 'enabled') AND NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = 'suppress') AND NEW.model NOT LIKE 'lexical-%'
    BEGIN
        INSERT INTO sync_outbox (change_id, entity, entity_id, op, detail, created_at)
        VALUES (lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))), 'embedding', NEW.memory_id, 'upsert', NEW.model, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
    END;
    "#,
];

/// Versión actual del esquema (la que tendrá `PRAGMA user_version` tras migrar).
pub const SCHEMA_VERSION: i64 = 3;

/// Conexión compartida. Las operaciones son cortas; nunca se mantiene el lock durante un `.await`.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    path: Option<std::path::PathBuf>,
}

impl Database {
    /// # Errors
    /// Si no se puede abrir el archivo o migrar.
    pub fn open(path: &Path) -> DbResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DbError::Other(e.to_string()))?;
        }
        let mut db = Self::init(Connection::open(path)?)?;
        db.path = Some(path.to_path_buf());
        Ok(db)
    }

    /// # Errors
    /// Si falla la migración.
    pub fn open_in_memory() -> DbResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> DbResult<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(usize::try_from(version).unwrap_or(0)) {
            conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {}; COMMIT;", i + 1))?;
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)), path: None })
    }

    /// Ruta del archivo (None en memoria).
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Versión del esquema de esta base.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn schema_version(&self) -> DbResult<i64> {
        Ok(self.conn().query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Comprobación rápida para el panel de estado.
    ///
    /// # Errors
    /// Si la base no responde.
    pub fn ping(&self) -> DbResult<()> {
        self.conn().query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }
}

#[cfg(test)]
mod migration_tests {
    use super::{Database, MIGRATIONS, SCHEMA_VERSION};
    use rusqlite::Connection;

    #[test]
    fn v1_databases_upgrade_without_losing_memories() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("v1.db");
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(&format!("BEGIN; {} PRAGMA user_version = 1; COMMIT;", MIGRATIONS[0])).expect("v1");
            conn.execute(
                "INSERT INTO memories (id, kind, content, created_at, source, relevance, confidence, refs, retention)
                 VALUES ('m1', 'semantic', 'Vivo en Guayaquil', '2026-01-01T00:00:00+00:00', 'user', 0.5, 1.0, '[]', '{\"kind\":\"permanent\"}')",
                [],
            )
            .expect("insert");
        }
        let db = Database::open(&path).expect("migrate");
        assert_eq!(db.schema_version().expect("v"), SCHEMA_VERSION);
        let (content, updated, verification): (String, String, String) = db
            .conn()
            .query_row("SELECT content, updated_at, verification FROM memories WHERE id = 'm1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("row");
        assert_eq!(content, "Vivo en Guayaquil");
        assert_eq!(updated, "2026-01-01T00:00:00+00:00");
        assert_eq!(verification, "verified", "lo escrito por el usuario queda verificado");
        for table in ["memory_relations", "memory_sources", "skills", "skill_versions", "memory_events", "embedding_metadata"] {
            let n: i64 = db.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).expect(table);
            assert_eq!(n, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Database, SCHEMA_VERSION};

    #[test]
    fn migrations_are_idempotent_across_reopens() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("j.db");
        {
            let db = Database::open(&path).expect("open");
            db.ping().expect("ping");
        }
        let db = Database::open(&path).expect("reopen");
        let v: i64 = db.conn().query_row("PRAGMA user_version", [], |r| r.get(0)).expect("v");
        assert_eq!(v, SCHEMA_VERSION);
    }
}
