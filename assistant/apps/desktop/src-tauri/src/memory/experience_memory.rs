//! Experiencias: lo que pasó en cada tarea con herramientas. Las repeticiones se agrupan.

use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use super::memory_engine::MemoryEngine;
use super::memory_repository::storage;
use super::secrets::redact_secrets;
use super::{MemoryError, MemoryResult};
use crate::domain::Experience;

/// Quita secretos de lo que se guarda como evidencia.
fn scrub(e: &Experience) -> Experience {
    let mut e = e.clone();
    e.goal = redact_secrets(&e.goal);
    e.result = redact_secrets(&e.result);
    e.errors = e.errors.iter().map(|x| redact_secrets(x)).collect();
    e.evidence = e
        .evidence
        .iter()
        .map(|v| serde_json::from_str(&redact_secrets(&v.to_string())).unwrap_or(serde_json::Value::Null))
        .collect();
    e
}

impl MemoryEngine {
    /// Guarda la experiencia. Si ya existe una con la misma firma (mismo objetivo y mismas
    /// herramientas), aumenta su contador y, si la nueva está verificada y la vieja no, la
    /// sustituye conservando id, contador y promoción.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub fn save_experience_dedup(&self, experience: &Experience) -> MemoryResult<(String, bool)> {
        let e = scrub(experience);
        let signature = e.signature();
        let conn = self.database().conn();
        let existing: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT id, data, occurrences FROM experiences WHERE signature = ?1 ORDER BY created_at LIMIT 1",
                [&signature],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage)?;
        let now = Utc::now().to_rfc3339();
        if let Some((id, data, occurrences)) = existing {
            let old: Experience = serde_json::from_str(&data).map_err(storage)?;
            let occurrences = u32::try_from(occurrences + 1).unwrap_or(u32::MAX);
            let mut merged = if e.verified && !old.verified { e.clone() } else { old.clone() };
            merged.id = id.clone();
            merged.promoted = old.promoted;
            merged.created_at = old.created_at;
            merged.occurrences = occurrences;
            conn.execute(
                "UPDATE experiences SET data = ?2, verified = ?3, occurrences = ?4, updated_at = ?5 WHERE id = ?1",
                params![id, serde_json::to_string(&merged).map_err(storage)?, merged.verified, occurrences, now],
            )
            .map_err(storage)?;
            return Ok((id, true));
        }
        conn.execute(
            "INSERT INTO experiences (id, data, verified, promoted, created_at, signature, occurrences, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![e.id, serde_json::to_string(&e).map_err(storage)?, e.verified, e.promoted, e.created_at.to_rfc3339(), signature, now],
        )
        .map_err(storage)?;
        Ok((e.id, false))
    }

    /// # Errors
    /// Errores de almacenamiento.
    pub fn experiences(&self, limit: usize) -> MemoryResult<Vec<Experience>> {
        let conn = self.database().conn();
        let mut stmt = conn
            .prepare("SELECT data, occurrences FROM experiences ORDER BY COALESCE(NULLIF(updated_at, ''), created_at) DESC LIMIT ?1")
            .map_err(storage)?;
        let rows = stmt
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (data, occ) = row.map_err(storage)?;
            if let Ok(mut e) = serde_json::from_str::<Experience>(&data) {
                e.occurrences = u32::try_from(occ).unwrap_or(1);
                out.push(e);
            }
        }
        Ok(out)
    }

    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn experience(&self, id: &str) -> MemoryResult<Experience> {
        let data: Option<String> = self
            .database()
            .conn()
            .query_row("SELECT data FROM experiences WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        serde_json::from_str(&data.ok_or(MemoryError::NotFound)?).map_err(storage)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::{ExperienceStep, VerificationStatus};
    use serde_json::json;

    pub(crate) fn experience(id: &str, goal: &str, verified: bool) -> Experience {
        let steps = vec![ExperienceStep {
            tool: "get_current_time".into(),
            arguments: json!({}),
            ok: true,
            verification: if verified { VerificationStatus::Passed } else { VerificationStatus::Inconclusive },
        }];
        let procedure = Experience::derive_procedure(&steps);
        Experience {
            id: id.into(),
            goal: goal.into(),
            context: String::new(),
            plan: String::new(),
            verified: procedure.is_some(),
            completed: true,
            procedure,
            steps,
            result: "ok".into(),
            evidence: vec![json!({"output": "mi password: abc123"})],
            errors: vec![],
            promoted: false,
            created_at: Utc::now(),
            occurrences: 1,
        }
    }

    #[test]
    fn repeated_tasks_are_grouped_and_upgraded_when_verified() {
        let e = MemoryEngine::for_tests().expect("e");
        let (a, dedup_a) = e.save_experience_dedup(&experience("e1", "¿Qué hora es?", false)).expect("a");
        let (b, dedup_b) = e.save_experience_dedup(&experience("e2", "qué hora es", true)).expect("b");
        assert!(!dedup_a && dedup_b);
        assert_eq!(a, b, "misma tarea, misma experiencia");
        let list = e.experiences(10).expect("l");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].occurrences, 2);
        assert!(list[0].verified, "se quedó con la versión verificada");
        assert!(!list[0].evidence[0].to_string().contains("abc123"), "evidencia sin secretos");
        e.save_experience_dedup(&experience("e3", "abre el bloc de notas", true)).expect("c");
        assert_eq!(e.experiences(10).expect("l").len(), 2);
    }
}
