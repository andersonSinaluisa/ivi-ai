//! Acceso a SQLite con **escritor único** (todas las operaciones se serializan con un mutex) y
//! transacciones que mantienen juntos el estado y su mensaje de outbox.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::model::{
    Approval, ApprovalState, ExecutionRecord, LocalState, NewTask, OutboxMessage, PendingApproval, StoreStats, TaskRow,
};
use crate::schema;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database is corrupt: {0}")]
    Corrupt(String),
    #[error("task {0} already has an execution")]
    AlreadyExecuted(Uuid),
    #[error("io error: {0}")]
    Io(String),
    #[error("invalid stored data: {0}")]
    Data(String),
}

type Result<T> = std::result::Result<T, StoreError>;

/// Resultado de finalizar una ejecución.
#[derive(Debug, Clone)]
pub struct FinishExecution {
    pub status: String,
    pub verification_status: String,
    pub result_json: String,
    pub error_code: Option<String>,
}

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
    db_created_at: DateTime<Utc>,
}

fn ts(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_ts(text: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| StoreError::Data(format!("invalid timestamp '{text}': {e}")))
}

fn parse_uuid(text: &str) -> Result<Uuid> {
    Uuid::try_parse(text).map_err(|e| StoreError::Data(format!("invalid uuid '{text}': {e}")))
}

impl Store {
    /// Abre (o crea) la base, aplica pragmas y migraciones y comprueba su integridad.
    ///
    /// # Errors
    /// [`StoreError::Corrupt`] si `quick_check` falla; el llamante decide reiniciar el estado.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_inner(path).map_err(|e| match e {
            // SQLite puede señalar la corrupción como error de cualquier sentencia, no solo en quick_check.
            StoreError::Sqlite(rusqlite::Error::SqliteFailure(f, message))
                if matches!(f.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) =>
            {
                StoreError::Corrupt(message.unwrap_or_else(|| f.to_string()))
            }
            other => other,
        })
    }

    fn open_inner(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Durabilidad ante corte de energía: un task.ack implica que la tarea está en disco.
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if check != "ok" {
            return Err(StoreError::Corrupt(check));
        }
        // Respaldo previo a una migración: si algo sale mal, queda la base anterior intacta.
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 0 && version < schema::CURRENT_VERSION {
            let backup = path.with_extension("db.bak");
            if let Err(error) = std::fs::copy(path, &backup) {
                tracing::warn!(%error, "cannot back up the database before migrating");
            } else {
                tracing::info!(from = version, to = schema::CURRENT_VERSION, backup = %backup.display(), "migrating");
            }
        }
        schema::migrate(&conn)?;

        let created: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'db_created_at'", [], |r| r.get(0)).optional()?;
        let db_created_at = match created {
            Some(text) => parse_ts(&text)?,
            None => {
                let now = Utc::now();
                conn.execute("INSERT INTO meta (key, value) VALUES ('db_created_at', ?1)", params![ts(now)])?;
                now
            }
        };
        Ok(Self { conn: Mutex::new(conn), path: path.to_path_buf(), db_created_at })
    }

    /// Abre la base; si está corrupta la aparta y crea una nueva (guardia `STATE_RESET`).
    ///
    /// # Errors
    /// Error de E/S al apartar el archivo o de SQLite al recrear.
    pub fn open_or_reset(path: &Path) -> Result<(Self, bool)> {
        match Self::open(path) {
            Ok(store) => Ok((store, false)),
            Err(StoreError::Corrupt(reason)) => {
                let backup = path.with_extension(format!("db.corrupt-{}", Utc::now().format("%Y%m%d%H%M%S")));
                tracing::error!(%reason, backup = %backup.display(), "local database is corrupt; starting a new one");
                std::fs::rename(path, &backup).map_err(|e| StoreError::Io(e.to_string()))?;
                for suffix in ["-wal", "-shm"] {
                    let side = PathBuf::from(format!("{}{suffix}", path.display()));
                    let _ = std::fs::remove_file(side);
                }
                Ok((Self::open(path)?, true))
            }
            Err(other) => Err(other),
        }
    }

    #[must_use]
    pub const fn db_created_at(&self) -> DateTime<Utc> {
        self.db_created_at
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        f(&conn)
    }

    fn with_tx<T>(&self, f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        let tx = conn.transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    // ---- tareas ---------------------------------------------------------------

    /// # Errors
    /// Error de SQLite o datos inválidos.
    pub fn get_task(&self, task_id: Uuid) -> Result<Option<TaskRow>> {
        self.with(|conn| {
            conn.query_row(&format!("{TASK_SELECT} AND task_id = ?1"), params![task_id.to_string()], row_to_task)
                .optional()
                .map_err(StoreError::from)?
                .transpose()
        })
    }

    /// Persiste una tarea nueva junto con sus mensajes de salida (una sola transacción).
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn insert_task(&self, task: &NewTask, outbox: &[OutboxMessage]) -> Result<()> {
        let payload_json = serde_json::to_string(&task.payload).unwrap_or_else(|_| "{}".into());
        let hash = crate::payload_hash(&task.payload);
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO tasks (task_id, correlation_id, task_type, tool, principal, payload_hash, payload_json,
                                    state, risk_level, decision, decision_reason, created_at, expires_at, received_at,
                                    terminal_at, max_duration_seconds, sensitive, signed_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    task.task_id.to_string(),
                    task.correlation_id.map(|c| c.to_string()),
                    task.task_type,
                    task.tool,
                    task.principal,
                    hash,
                    payload_json,
                    task.state.as_str(),
                    task.risk_level,
                    task.decision,
                    task.decision_reason,
                    ts(task.created_at),
                    ts(task.expires_at),
                    ts(Utc::now()),
                    task.state.is_terminal().then(|| ts(Utc::now())),
                    task.max_duration_seconds,
                    i32::from(task.sensitive),
                    task.signed_by,
                ],
            )?;
            push_outbox(tx, task.task_id, outbox)
        })
    }

    /// Cambia el estado de una tarea y encola sus mensajes en la misma transacción.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn set_state(&self, task_id: Uuid, state: LocalState, outbox: &[OutboxMessage]) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE tasks SET state = ?2, terminal_at = COALESCE(?3, terminal_at) WHERE task_id = ?1",
                params![task_id.to_string(), state.as_str(), state.is_terminal().then(|| ts(Utc::now()))],
            )?;
            push_outbox(tx, task_id, outbox)
        })
    }

    /// Marca `Running` y registra la ejecución. Falla si la tarea ya tuvo una (garantía W4/W9).
    ///
    /// # Errors
    /// [`StoreError::AlreadyExecuted`] si ya existe una ejecución para esa tarea.
    pub fn start_execution(
        &self,
        task_id: Uuid,
        execution_id: Uuid,
        started_at: DateTime<Utc>,
        outbox: &[OutboxMessage],
    ) -> Result<()> {
        self.with_tx(|tx| {
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO executions (execution_id, task_id, status, verification_status, started_at)
                 VALUES (?1, ?2, 'RUNNING', 'PENDING', ?3)",
                params![execution_id.to_string(), task_id.to_string(), ts(started_at)],
            )?;
            if inserted == 0 {
                return Err(StoreError::AlreadyExecuted(task_id));
            }
            tx.execute(
                "UPDATE tasks SET state = ?2 WHERE task_id = ?1",
                params![task_id.to_string(), LocalState::Running.as_str()],
            )?;
            push_outbox(tx, task_id, outbox)
        })
    }

    /// # Errors
    /// Error de SQLite.
    pub fn set_verifying(&self, task_id: Uuid) -> Result<()> {
        self.set_state(task_id, LocalState::Verifying, &[])
    }

    /// Guarda el resultado, el estado terminal y el mensaje a publicar en una transacción.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn finish_execution(
        &self,
        task_id: Uuid,
        state: LocalState,
        execution: &FinishExecution,
        outbox: &[OutboxMessage],
    ) -> Result<()> {
        let size = i64::try_from(execution.result_json.len()).unwrap_or(i64::MAX);
        let hash = format!("sha256:{:x}", <sha2::Sha256 as sha2::Digest>::digest(execution.result_json.as_bytes()));
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE executions SET status = ?2, verification_status = ?3, finished_at = ?4, result_json = ?5,
                        result_sha256 = ?6, result_size = ?7, error_code = ?8
                 WHERE task_id = ?1",
                params![
                    task_id.to_string(),
                    execution.status,
                    execution.verification_status,
                    ts(Utc::now()),
                    execution.result_json,
                    hash,
                    size,
                    execution.error_code,
                ],
            )?;
            tx.execute(
                "UPDATE tasks SET state = ?2, terminal_at = ?3 WHERE task_id = ?1",
                params![task_id.to_string(), state.as_str(), ts(Utc::now())],
            )?;
            push_outbox(tx, task_id, outbox)
        })
    }

    /// # Errors
    /// Error de SQLite.
    pub fn execution(&self, task_id: Uuid) -> Result<Option<ExecutionRecord>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT execution_id, task_id, status, verification_status, started_at, finished_at, result_json, error_code
                 FROM executions WHERE task_id = ?1",
                params![task_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(StoreError::from)?
            .map(|(id, task, status, verification, started, finished, result, error)| {
                Ok(ExecutionRecord {
                    execution_id: parse_uuid(&id)?,
                    task_id: parse_uuid(&task)?,
                    status,
                    verification_status: verification,
                    started_at: parse_ts(&started)?,
                    finished_at: finished.as_deref().map(parse_ts).transpose()?,
                    result_json: result,
                    error_code: error,
                })
            })
            .transpose()
        })
    }

    /// Tareas en estados no terminales dados (recuperación de arranque).
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn tasks_in(&self, states: &[LocalState]) -> Result<Vec<TaskRow>> {
        self.with(|conn| {
            let mut out = Vec::new();
            for state in states {
                let mut stmt = conn.prepare(&format!("{TASK_SELECT} AND state = ?1"))?;
                let rows = stmt.query_map(params![state.as_str()], row_to_task)?;
                for row in rows {
                    out.push(row??);
                }
            }
            Ok(out)
        })
    }

    /// `openTaskIds` del hello: no terminales + terminales sin `task.update.ack`, los más recientes.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn open_task_ids(&self, limit: usize) -> Result<Vec<Uuid>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT task_id FROM tasks WHERE settled = 0 ORDER BY received_at DESC LIMIT ?1")?;
            let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(256)], |row| row.get::<_, String>(0))?;
            rows.map(|r| parse_uuid(&r?)).collect()
        })
    }

    // ---- outbox ---------------------------------------------------------------

    /// Mensajes aún no publicados, en orden.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn pending_outbox(&self) -> Result<Vec<(i64, String)>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT seq, envelope_json FROM outbox WHERE published_at IS NULL ORDER BY seq")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Último mensaje de cada tarea no *settled* (reenvío tras reconectar y cada minuto).
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn resend_candidates(&self) -> Result<Vec<(i64, String)>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT o.seq, o.envelope_json FROM outbox o
                 JOIN tasks t ON t.task_id = o.task_id
                 WHERE t.settled = 0 AND o.acked_at IS NULL
                   AND o.seq = (SELECT MAX(seq) FROM outbox WHERE task_id = o.task_id)
                 ORDER BY o.seq",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Último mensaje persistido de una tarea (respuesta a un dispatch duplicado).
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn last_message(&self, task_id: Uuid) -> Result<Option<String>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT envelope_json FROM outbox WHERE task_id = ?1 ORDER BY seq DESC LIMIT 1",
                params![task_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    /// # Errors
    /// Error de SQLite.
    pub fn mark_published(&self, seq: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE outbox SET published_at = ?2, attempts = attempts + 1, last_attempt_at = ?2 WHERE seq = ?1",
                params![seq, ts(Utc::now())],
            )?;
            Ok(())
        })
    }

    /// `task.update.ack`: marca la tarea *settled* y sus mensajes confirmados.
    /// Devuelve `true` si la tarea era sensible (hay que redactar su resultado).
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn settle(&self, task_id: Uuid) -> Result<bool> {
        self.with_tx(|tx| {
            let sensitive: Option<i64> = tx
                .query_row("SELECT sensitive FROM tasks WHERE task_id = ?1", params![task_id.to_string()], |r| r.get(0))
                .optional()?;
            let Some(sensitive) = sensitive else { return Ok(false) };
            tx.execute(
                "UPDATE tasks SET settled = 1, settled_at = ?2 WHERE task_id = ?1",
                params![task_id.to_string(), ts(Utc::now())],
            )?;
            tx.execute(
                "UPDATE outbox SET acked_at = ?2 WHERE task_id = ?1 AND acked_at IS NULL",
                params![task_id.to_string(), ts(Utc::now())],
            )?;
            Ok(sensitive == 1)
        })
    }

    /// Sustituye el resultado y el payload por su versión redactada (tras *settled*).
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn redact(&self, task_id: Uuid, redacted_result: &str, redacted_payload: &str) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE executions SET result_json = ?2 WHERE task_id = ?1",
                params![task_id.to_string(), redacted_result],
            )?;
            tx.execute(
                "UPDATE tasks SET payload_json = ?2 WHERE task_id = ?1",
                params![task_id.to_string(), redacted_payload],
            )?;
            Ok(())
        })
    }

    // ---- aprobaciones ------------------------------------------------------------

    /// # Errors
    /// Error de SQLite.
    pub fn create_approval(&self, task_id: Uuid, payload_hash: &str, expires_at: DateTime<Utc>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO approvals (task_id, state, payload_hash, expires_at) VALUES (?1, 'Pending', ?2, ?3)",
                params![task_id.to_string(), payload_hash, ts(expires_at)],
            )?;
            Ok(())
        })
    }

    /// # Errors
    /// Error de SQLite.
    pub fn approval(&self, task_id: Uuid) -> Result<Option<Approval>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT task_id, state, payload_hash, expires_at, decided_at, decided_by FROM approvals WHERE task_id = ?1",
                params![task_id.to_string()],
                row_to_approval,
            )
            .optional()
            .map_err(StoreError::from)?
            .transpose()
        })
    }

    /// Aprobaciones pendientes con el contexto de su tarea.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn pending_approvals(&self) -> Result<Vec<PendingApproval>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT a.task_id, a.state, a.payload_hash, a.expires_at, a.decided_at, a.decided_by,
                        t.tool, t.task_type, t.principal, t.risk_level, t.payload_json
                 FROM approvals a JOIN tasks t ON t.task_id = a.task_id
                 WHERE a.state = 'Pending' ORDER BY a.expires_at",
            )?;
            let rows = stmt.query_map([], |row| {
                let approval = row_to_approval(row)?;
                Ok((
                    approval,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (approval, tool, task_type, principal, risk_level, payload) = row?;
                out.push(PendingApproval {
                    approval: approval?,
                    tool,
                    task_type,
                    principal,
                    risk_level,
                    payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
                });
            }
            Ok(out)
        })
    }

    /// Decide una aprobación pendiente. `false` si ya no estaba pendiente.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn decide_approval(&self, task_id: Uuid, state: ApprovalState, by: &str) -> Result<bool> {
        self.with(|conn| {
            let updated = conn.execute(
                "UPDATE approvals SET state = ?2, decided_at = ?3, decided_by = ?4 WHERE task_id = ?1 AND state = 'Pending'",
                params![task_id.to_string(), state.as_str(), ts(Utc::now()), by],
            )?;
            Ok(updated == 1)
        })
    }

    /// Aprobaciones pendientes ya vencidas.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn expired_approvals(&self, now: DateTime<Utc>) -> Result<Vec<Uuid>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT task_id FROM approvals WHERE state = 'Pending' AND expires_at <= ?1")?;
            let rows = stmt.query_map(params![ts(now)], |row| row.get::<_, String>(0))?;
            rows.map(|r| parse_uuid(&r?)).collect()
        })
    }

    // ---- mantenimiento ------------------------------------------------------------

    /// Borra tareas *settled* más antiguas que la retención y outbox confirmado hace > 24 h.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn cleanup(&self, retention_days: i64, now: DateTime<Utc>) -> Result<usize> {
        let cutoff = ts(now - chrono::Duration::days(retention_days));
        let outbox_cutoff = ts(now - chrono::Duration::hours(24));
        self.with_tx(|tx| {
            tx.execute("DELETE FROM outbox WHERE acked_at IS NOT NULL AND acked_at < ?1", params![outbox_cutoff])?;
            // Los nonces se conservan tanto como las tareas: más allá, la tarea ya no existe.
            tx.execute("DELETE FROM used_nonces WHERE used_at < ?1", params![cutoff])?;
            let removed = tx.execute(
                "DELETE FROM tasks WHERE task_id IN (SELECT task_id FROM tasks WHERE settled = 1 AND settled_at < ?1 LIMIT 500)",
                params![cutoff],
            )?;
            Ok(removed)
        })
    }

    /// Marca un nonce de decisión de aprobación como usado. `false` si ya lo estaba.
    ///
    /// Es la barrera anti-replay: una decisión firmada capturada no puede volver a aplicarse
    /// (HU-WIN-062). La unicidad la impone la clave primaria, no el código.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn consume_nonce(&self, nonce: &str, task_id: Uuid, now: DateTime<Utc>) -> Result<bool> {
        self.with_tx(|tx| {
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO used_nonces (nonce, task_id, used_at) VALUES (?1, ?2, ?3)",
                params![nonce, task_id.to_string(), ts(now)],
            )?;
            Ok(inserted == 1)
        })
    }

    /// Lee un valor de `meta`.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.with(|conn| {
            conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| row.get(0))
                .optional()
                .map_err(StoreError::from)
        })
    }

    /// Escribe un valor de `meta`.
    ///
    /// # Errors
    /// Error de SQLite.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![key, value])?;
            Ok(())
        })
    }

    /// # Errors
    /// Error de SQLite.
    pub fn stats(&self) -> Result<StoreStats> {
        self.with(|conn| {
            let tasks: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
            let pending: i64 =
                conn.query_row("SELECT COUNT(*) FROM outbox WHERE published_at IS NULL", [], |r| r.get(0))?;
            let unsettled: i64 = conn.query_row("SELECT COUNT(*) FROM tasks WHERE settled = 0", [], |r| r.get(0))?;
            let oldest: Option<String> = conn
                .query_row("SELECT MIN(received_at) FROM tasks WHERE settled = 0", [], |r| r.get(0))
                .optional()?
                .flatten();
            let size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
            Ok(StoreStats {
                tasks: u64::try_from(tasks).unwrap_or(0),
                outbox_pending: u64::try_from(pending).unwrap_or(0),
                unsettled: u64::try_from(unsettled).unwrap_or(0),
                oldest_unsettled_age_seconds: oldest
                    .as_deref()
                    .map(parse_ts)
                    .transpose()?
                    .map(|t| (Utc::now() - t).num_seconds()),
                size_bytes: size,
            })
        })
    }
}

const TASK_SELECT: &str = "SELECT task_id, correlation_id, task_type, tool, principal, payload_json, payload_hash,
                                  state, settled, created_at, expires_at, max_duration_seconds, sensitive,
                                  risk_level, decision, signed_by
                           FROM tasks WHERE 1 = 1";

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Result<TaskRow>> {
    let task_id: String = row.get(0)?;
    let correlation: Option<String> = row.get(1)?;
    let payload: String = row.get(5)?;
    let state: String = row.get(7)?;
    let created_at: String = row.get(9)?;
    let expires_at: String = row.get(10)?;
    let max_duration: Option<i64> = row.get(11)?;
    let sensitive: i64 = row.get(12)?;
    let task_type: String = row.get(2)?;
    let tool: Option<String> = row.get(3)?;
    let principal: String = row.get(4)?;
    let payload_hash: String = row.get(6)?;
    let settled: i64 = row.get(8)?;
    let risk_level: Option<String> = row.get(13)?;
    let decision: Option<String> = row.get(14)?;
    let signed_by: Option<String> = row.get(15)?;
    Ok((|| {
        Ok(TaskRow {
            task_id: parse_uuid(&task_id)?,
            correlation_id: correlation.as_deref().map(parse_uuid).transpose()?,
            task_type,
            tool,
            principal,
            payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
            payload_hash,
            state: LocalState::parse(&state).ok_or_else(|| StoreError::Data(format!("unknown state '{state}'")))?,
            settled: settled == 1,
            risk_level,
            decision,
            signed_by,
            created_at: parse_ts(&created_at)?,
            expires_at: parse_ts(&expires_at)?,
            max_duration_seconds: max_duration.and_then(|v| u32::try_from(v).ok()),
            sensitive: sensitive == 1,
        })
    })())
}

fn row_to_approval(row: &Row<'_>) -> rusqlite::Result<Result<Approval>> {
    let task_id: String = row.get(0)?;
    let state: String = row.get(1)?;
    let payload_hash: String = row.get(2)?;
    let expires_at: String = row.get(3)?;
    let decided_at: Option<String> = row.get(4)?;
    let decided_by: Option<String> = row.get(5)?;
    Ok((|| {
        Ok(Approval {
            task_id: parse_uuid(&task_id)?,
            state: ApprovalState::parse(&state)
                .ok_or_else(|| StoreError::Data(format!("unknown approval state '{state}'")))?,
            payload_hash,
            expires_at: parse_ts(&expires_at)?,
            decided_at: decided_at.as_deref().map(parse_ts).transpose()?,
            decided_by,
        })
    })())
}

fn push_outbox(tx: &rusqlite::Transaction<'_>, task_id: Uuid, messages: &[OutboxMessage]) -> Result<()> {
    for message in messages {
        tx.execute(
            "INSERT INTO outbox (task_id, message_type, envelope_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![task_id.to_string(), message.message_type, message.envelope_json, ts(Utc::now())],
        )?;
    }
    Ok(())
}
