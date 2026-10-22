//! Ejecuciones del agente (pantalla de Tareas).

use chrono::Utc;
use rusqlite::params;
use serde::Serialize;

use super::{Database, DbResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub conversation_id: String,
    pub goal: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub tool_calls: i64,
}

impl Database {
    /// # Errors
    /// Errores de SQLite.
    pub fn start_run(&self, id: &str, conversation_id: &str, goal: &str) -> DbResult<()> {
        let goal: String = goal.chars().take(300).collect();
        self.conn().execute(
            "INSERT INTO runs (id, conversation_id, goal, status, started_at) VALUES (?1, ?2, ?3, 'running', ?4)",
            params![id, conversation_id, crate::memory::secrets::redact_secrets(&goal), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn finish_run(&self, id: &str, status: &str, error: Option<&str>, tool_calls: usize) -> DbResult<()> {
        self.conn().execute(
            "UPDATE runs SET status = ?2, finished_at = ?3, error = ?4, tool_calls = ?5 WHERE id = ?1",
            params![id, status, Utc::now().to_rfc3339(), error, i64::try_from(tool_calls).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    /// Ejecuciones que quedaron `running` por un cierre abrupto pasan a `failed`.
    ///
    /// # Errors
    /// Errores de SQLite.
    pub fn recover_interrupted_runs(&self) -> DbResult<usize> {
        Ok(self.conn().execute(
            "UPDATE runs SET status = 'failed', error = 'interrumpida al cerrar la aplicación', finished_at = ?1
             WHERE status = 'running'",
            [Utc::now().to_rfc3339()],
        )?)
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn list_runs(&self, limit: usize) -> DbResult<Vec<RunSummary>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, goal, status, started_at, finished_at, error, tool_calls
             FROM runs ORDER BY started_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            Ok(RunSummary {
                id: r.get(0)?,
                conversation_id: r.get(1)?,
                goal: r.get(2)?,
                status: r.get(3)?,
                started_at: r.get(4)?,
                finished_at: r.get(5)?,
                error: r.get(6)?,
                tool_calls: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_runs_are_recovered_as_failed() {
        let db = Database::open_in_memory().expect("db");
        db.start_run("r1", "c1", "abre notepad").expect("start");
        db.start_run("r2", "c1", "hora").expect("start");
        db.finish_run("r2", "completed", None, 1).expect("finish");
        assert_eq!(db.recover_interrupted_runs().expect("recover"), 1);
        let runs = db.list_runs(10).expect("list");
        let r1 = runs.iter().find(|r| r.id == "r1").expect("r1");
        assert_eq!(r1.status, "failed");
    }
}
