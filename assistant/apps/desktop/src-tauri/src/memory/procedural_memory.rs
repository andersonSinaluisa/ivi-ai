//! Habilidades: procedimientos reutilizables con versiones. Solo nacen de experiencias
//! verificadas y completadas, y nunca se ejecutan solas.

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::memory_engine::MemoryEngine;
use super::memory_repository::storage;
use super::{MemoryError, MemoryResult, ProceduralMemory};
use crate::domain::{Experience, MemoryKind, NewMemory, NewSource, Skill, SkillVersion, SourceKind, VisualEvent};

const SKILL_COLUMNS: &str =
    "id, name, description, procedure, enabled, requires_review, source, experience_id, current_version, created_at, updated_at";

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc))
}

pub(crate) fn row_to_skill(r: &Row<'_>) -> rusqlite::Result<Skill> {
    Ok(Skill {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        procedure: r.get(3)?,
        enabled: r.get::<_, i64>(4)? != 0,
        requires_review: r.get::<_, i64>(5)? != 0,
        source: r.get(6)?,
        experience_id: r.get(7)?,
        current_version: r.get(8)?,
        created_at: t(&r.get::<_, String>(9)?),
        updated_at: t(&r.get::<_, String>(10)?),
    })
}

impl MemoryEngine {
    /// # Errors
    /// Errores de almacenamiento.
    pub fn skills(&self) -> MemoryResult<Vec<Skill>> {
        let conn = self.database().conn();
        let mut stmt = conn.prepare(&format!("SELECT {SKILL_COLUMNS} FROM skills ORDER BY updated_at DESC")).map_err(storage)?;
        let rows = stmt.query_map([], row_to_skill).map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn skill(&self, id: &str) -> MemoryResult<Skill> {
        self.database()
            .conn()
            .query_row(&format!("SELECT {SKILL_COLUMNS} FROM skills WHERE id = ?1"), [id], row_to_skill)
            .optional()
            .map_err(storage)?
            .ok_or(MemoryError::NotFound)
    }

    /// # Errors
    /// Errores de almacenamiento.
    pub fn skill_versions(&self, id: &str) -> MemoryResult<Vec<SkillVersion>> {
        let conn = self.database().conn();
        let mut stmt = conn
            .prepare("SELECT skill_id, version, procedure, notes, created_at FROM skill_versions WHERE skill_id = ?1 ORDER BY version DESC")
            .map_err(storage)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(SkillVersion {
                    skill_id: r.get(0)?,
                    version: r.get(1)?,
                    procedure: r.get(2)?,
                    notes: r.get(3)?,
                    created_at: t(&r.get::<_, String>(4)?),
                })
            })
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }

    /// Activar una habilidad importada es la revisión del usuario.
    ///
    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn set_skill_enabled(&self, id: &str, enabled: bool) -> MemoryResult<Skill> {
        let n = self
            .database()
            .conn()
            .execute(
                "UPDATE skills SET enabled = ?2, requires_review = CASE WHEN ?2 = 1 THEN 0 ELSE requires_review END, updated_at = ?3 WHERE id = ?1",
                params![id, i64::from(enabled), Utc::now().to_rfc3339()],
            )
            .map_err(storage)?;
        if n == 0 {
            return Err(MemoryError::NotFound);
        }
        self.skill(id)
    }

    /// Corrige el procedimiento creando una versión nueva (la anterior se conserva).
    ///
    /// # Errors
    /// [`MemoryError::NotFound`] o procedimiento vacío o con secretos.
    pub fn update_skill_procedure(&self, id: &str, procedure: &str, notes: Option<&str>) -> MemoryResult<Skill> {
        let procedure = procedure.trim();
        if procedure.is_empty() {
            return Err(MemoryError::Invalid("el procedimiento está vacío".into()));
        }
        if super::secrets::contains_secret(procedure) {
            return Err(MemoryError::ContainsSecret);
        }
        let skill = self.skill(id)?;
        let version = skill.current_version + 1;
        let now = Utc::now().to_rfc3339();
        let conn = self.database().conn();
        conn.execute(
            "INSERT INTO skill_versions (skill_id, version, procedure, notes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, version, procedure, notes, now],
        )
        .map_err(storage)?;
        conn.execute(
            "UPDATE skills SET procedure = ?2, current_version = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, procedure, version, now],
        )
        .map_err(storage)?;
        drop(conn);
        self.skill(id)
    }

    fn promote_inner(&self, experience_id: &str) -> MemoryResult<Skill> {
        let mut exp: Experience = self.experience(experience_id)?;
        if exp.promoted {
            let existing: Option<String> = self
                .database()
                .conn()
                .query_row("SELECT id FROM skills WHERE experience_id = ?1", [experience_id], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            if let Some(id) = existing {
                return self.skill(&id);
            }
        }
        let procedure = match (&exp.procedure, exp.verified, exp.completed) {
            (Some(p), true, true) => p.clone(),
            _ => return Err(MemoryError::NotVerified),
        };
        let now = Utc::now();
        let skill = Skill {
            id: uuid::Uuid::new_v4().to_string(),
            name: exp.goal.chars().take(80).collect(),
            description: format!("Aprendida de una tarea verificada ({} vez/veces).", exp.occurrences),
            procedure: procedure.clone(),
            enabled: true,
            requires_review: false,
            source: "experience".into(),
            experience_id: Some(exp.id.clone()),
            current_version: 1,
            created_at: now,
            updated_at: now,
        };
        {
            let conn = self.database().conn();
            conn.execute(
                &format!("INSERT INTO skills ({SKILL_COLUMNS}) VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?6, 1, ?7, ?7)"),
                params![skill.id, skill.name, skill.description, skill.procedure, skill.source, skill.experience_id, now.to_rfc3339()],
            )
            .map_err(storage)?;
            conn.execute(
                "INSERT INTO skill_versions (skill_id, version, procedure, notes, created_at) VALUES (?1, 1, ?2, 'creada desde una experiencia verificada', ?3)",
                params![skill.id, procedure, now.to_rfc3339()],
            )
            .map_err(storage)?;
            exp.promoted = true;
            conn.execute(
                "UPDATE experiences SET data = ?2, promoted = 1 WHERE id = ?1",
                params![exp.id, serde_json::to_string(&exp).map_err(storage)?],
            )
            .map_err(storage)?;
        }
        self.repo.log_event("SKILL_LEARNED", None, &[], Some(&skill.name))?;
        self.emit(VisualEvent::SkillLearned { skill_id: skill.id.clone(), name: skill.name.clone() });
        Ok(skill)
    }

    /// Promueve y además deja el procedimiento como memoria procedimental consultable.
    ///
    /// # Errors
    /// [`MemoryError::NotVerified`] si la experiencia no está verificada y completada.
    pub async fn promote_experience(&self, experience_id: &str) -> MemoryResult<Skill> {
        let skill = self.promote_inner(experience_id)?;
        let mut note = NewMemory::user(MemoryKind::Procedural, format!("Para \"{}\":\n{}", skill.name, skill.procedure));
        note.summary = Some(format!("Habilidad: {}", skill.name));
        note.source = NewSource { kind: SourceKind::Experience, reference: Some(experience_id.to_owned()), label: Some("experiencia verificada".into()) };
        note.tags = vec!["habilidad".into()];
        if let Err(e) = self.remember(note).await {
            tracing::warn!(error = %e, "habilidad creada sin recuerdo procedimental");
        }
        Ok(skill)
    }
}

impl ProceduralMemory for MemoryEngine {
    fn save_experience(&self, experience: &Experience) -> MemoryResult<String> {
        self.save_experience_dedup(experience).map(|(id, _)| id)
    }

    fn list_experiences(&self, limit: usize) -> MemoryResult<Vec<Experience>> {
        self.experiences(limit)
    }

    fn promote(&self, experience_id: &str) -> MemoryResult<Skill> {
        self.promote_inner(experience_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::experience_memory::tests::experience;

    #[tokio::test]
    async fn only_verified_completed_experiences_become_versioned_skills() {
        let e = MemoryEngine::for_tests().expect("e");
        e.save_experience_dedup(&experience("bad", "tarea dudosa", false)).expect("bad");
        assert!(matches!(e.promote_experience("bad").await, Err(MemoryError::NotVerified)));
        let mut incomplete = experience("inc", "tarea a medias", true);
        incomplete.completed = false;
        e.save_experience_dedup(&incomplete).expect("inc");
        assert!(matches!(e.promote_experience("inc").await, Err(MemoryError::NotVerified)));

        e.save_experience_dedup(&experience("good", "saber la hora", true)).expect("good");
        let mut rx = e.subscribe();
        let skill = e.promote_experience("good").await.expect("skill");
        assert!(skill.enabled && skill.current_version == 1);
        assert_eq!(e.promote_experience("good").await.expect("again").id, skill.id, "no se duplica");
        assert!(matches!(rx.try_recv().expect("ev"), VisualEvent::SkillLearned { .. }));

        let v2 = e.update_skill_procedure(&skill.id, "1. get_current_time {}\n2. responder en una frase", Some("más corta")).expect("v2");
        assert_eq!(v2.current_version, 2);
        assert_eq!(e.skill_versions(&skill.id).expect("v").len(), 2);
        let procedural = e.list_memories(&crate::memory::memory_repository::MemoryFilter { kind: Some(MemoryKind::Procedural), ..Default::default() }, 0, 10).expect("p");
        assert_eq!(procedural.total, 1);
    }
}
