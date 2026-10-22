//! Registro de auditoría: cada decisión de política y cada ejecución de herramienta.

use chrono::Utc;
use rusqlite::params;
use serde::Serialize;

use super::{Database, DbResult};
use crate::memory::secrets::redact_secrets;

#[derive(Debug, Clone, Default)]
pub struct AuditRecord {
    pub run_id: Option<String>,
    pub actor: String,
    pub action: String,
    pub tool: Option<String>,
    pub risk: Option<String>,
    pub decision: Option<String>,
    pub outcome: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub ts: String,
    pub run_id: Option<String>,
    pub actor: String,
    pub action: String,
    pub tool: Option<String>,
    pub risk: Option<String>,
    pub decision: Option<String>,
    pub outcome: Option<String>,
    pub detail: Option<String>,
}

/// Longitud máxima del detalle auditado (los argumentos pueden ser grandes).
const MAX_DETAIL: usize = 2000;

impl Database {
    /// # Errors
    /// Errores de SQLite.
    pub fn audit(&self, r: &AuditRecord) -> DbResult<()> {
        let detail = r.detail.as_deref().map(|d| {
            let d = redact_secrets(d);
            if d.chars().count() > MAX_DETAIL { d.chars().take(MAX_DETAIL).collect::<String>() + "…" } else { d }
        });
        self.conn().execute(
            "INSERT INTO audit_log (ts, run_id, actor, action, tool, risk, decision, outcome, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![Utc::now().to_rfc3339(), r.run_id, r.actor, r.action, r.tool, r.risk, r.decision, r.outcome, detail],
        )?;
        Ok(())
    }

    /// # Errors
    /// Errores de SQLite.
    pub fn list_audit(&self, limit: usize) -> DbResult<Vec<AuditEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, run_id, actor, action, tool, risk, decision, outcome, detail
             FROM audit_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                ts: r.get(1)?,
                run_id: r.get(2)?,
                actor: r.get(3)?,
                action: r.get(4)?,
                tool: r.get(5)?,
                risk: r.get(6)?,
                decision: r.get(7)?,
                outcome: r.get(8)?,
                detail: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
