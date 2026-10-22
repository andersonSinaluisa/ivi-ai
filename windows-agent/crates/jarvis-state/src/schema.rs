//! Migraciones embebidas, versionadas con `user_version`. Solo hacia adelante y aditivas.

use rusqlite::Connection;

pub const CURRENT_VERSION: i64 = 2;

const V1: &str = r"
CREATE TABLE tasks (
    task_id              TEXT PRIMARY KEY,
    correlation_id       TEXT NULL,
    task_type            TEXT NOT NULL,
    tool                 TEXT NULL,
    principal            TEXT NOT NULL,
    payload_hash         TEXT NOT NULL,
    payload_json         TEXT NOT NULL,
    state                TEXT NOT NULL,
    risk_level           TEXT NULL,
    decision             TEXT NULL,
    decision_reason      TEXT NULL,
    created_at           TEXT NOT NULL,
    expires_at           TEXT NOT NULL,
    received_at          TEXT NOT NULL,
    terminal_at          TEXT NULL,
    settled              INTEGER NOT NULL DEFAULT 0,
    settled_at           TEXT NULL,
    max_duration_seconds INTEGER NULL,
    sensitive            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_tasks_state ON tasks (state);
CREATE INDEX ix_tasks_settled ON tasks (settled, terminal_at);

-- Como máximo una ejecución por tarea: garantía estructural, no solo de código.
CREATE TABLE executions (
    execution_id        TEXT PRIMARY KEY,
    task_id             TEXT NOT NULL UNIQUE REFERENCES tasks (task_id) ON DELETE CASCADE,
    status              TEXT NOT NULL,
    verification_status TEXT NOT NULL,
    started_at          TEXT NOT NULL,
    finished_at         TEXT NULL,
    result_json         TEXT NULL,
    result_sha256       TEXT NULL,
    result_size         INTEGER NULL,
    error_code          TEXT NULL
);

CREATE TABLE approvals (
    task_id       TEXT PRIMARY KEY REFERENCES tasks (task_id) ON DELETE CASCADE,
    state         TEXT NOT NULL,
    payload_hash  TEXT NOT NULL,
    expires_at    TEXT NOT NULL,
    decided_at    TEXT NULL,
    decided_by    TEXT NULL
);

CREATE TABLE outbox (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL REFERENCES tasks (task_id) ON DELETE CASCADE,
    message_type    TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT NULL,
    published_at    TEXT NULL,
    acked_at        TEXT NULL
);
CREATE INDEX ix_outbox_pending ON outbox (published_at, seq);
CREATE INDEX ix_outbox_task ON outbox (task_id, seq);

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// v2 (P8): origen probado de la tarea y nonces ya usados de las decisiones remotas.
const V2: &str = r"
ALTER TABLE tasks ADD COLUMN signed_by TEXT NULL;

-- Una decisión de aprobación firmada solo vale una vez: capturarla no sirve para repetirla.
CREATE TABLE used_nonces (
    nonce   TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    used_at TEXT NOT NULL
);
CREATE INDEX ix_used_nonces_used_at ON used_nonces (used_at);
";

/// Aplica las migraciones pendientes.
///
/// # Errors
/// Error de SQLite.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(V1)?;
    }
    if version < 2 {
        conn.execute_batch(V2)?;
    }
    conn.pragma_update(None, "user_version", CURRENT_VERSION)?;
    Ok(())
}
