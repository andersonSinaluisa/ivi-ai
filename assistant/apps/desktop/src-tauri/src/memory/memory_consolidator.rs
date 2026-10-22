//! Consolidación: aplica la retención, fusiona duplicados y poda relaciones inferidas débiles.

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::params;
use serde::Serialize;

use super::memory_engine::MemoryEngine;
use super::memory_repository::storage;
use super::MemoryResult;
use crate::domain::{MemoryRecord, Provenance, Verification, VisualEvent};

/// Relaciones inferidas, no verificadas y por debajo de esta confianza se eliminan.
pub const PRUNE_BELOW: f32 = 0.35;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidationReport {
    pub removed_expired: usize,
    pub merged_duplicates: usize,
    pub pruned_relations: usize,
}

fn keeper_rank(m: &MemoryRecord) -> (u8, i64, i64) {
    let v = match m.verification {
        Verification::Verified => 2,
        Verification::Unverified => 1,
        Verification::Disputed => 0,
    };
    #[allow(clippy::cast_possible_truncation)]
    let conf = (m.confidence * 1000.0) as i64;
    // A igualdad, el más antiguo (su id es el que otros ya referencian).
    (v, conf, -m.created_at.timestamp())
}

impl MemoryEngine {
    /// # Errors
    /// Errores de almacenamiento.
    pub fn consolidate(&self) -> MemoryResult<ConsolidationReport> {
        let mut report = ConsolidationReport::default();
        let now = Utc::now();
        let all = self.repo.recent(usize::MAX >> 1, 0)?;

        // 1. Retención.
        for m in all.iter().filter(|m| m.is_expired(now)) {
            if self.repo.delete(&m.id)? {
                self.cache_remove(&m.id);
                report.removed_expired += 1;
            }
        }

        // 2. Duplicados por contenido: se conserva el mejor y se le pasan las relaciones.
        let mut groups: HashMap<&str, Vec<&MemoryRecord>> = HashMap::new();
        for m in all.iter().filter(|m| !m.is_expired(now) && !m.content_hash.is_empty()) {
            groups.entry(m.content_hash.as_str()).or_default().push(m);
        }
        for (_, mut group) in groups.into_iter().filter(|(_, g)| g.len() > 1) {
            group.sort_by_key(|m| std::cmp::Reverse(keeper_rank(m)));
            let keeper = group[0].id.clone();
            for dup in &group[1..] {
                let conn = self.database().conn();
                // Mover relaciones al recuerdo que se queda (evitando bucles y duplicadas).
                conn.execute(
                    "INSERT OR IGNORE INTO memory_relations (id, source_id, target_id, kind, confidence, provenance, verified, note, created_at)
                     SELECT id || '-m', CASE WHEN source_id = ?1 THEN ?2 ELSE source_id END,
                            CASE WHEN target_id = ?1 THEN ?2 ELSE target_id END,
                            kind, confidence, provenance, verified, note, created_at
                     FROM memory_relations WHERE (source_id = ?1 OR target_id = ?1)
                       AND (CASE WHEN source_id = ?1 THEN ?2 ELSE source_id END) <> (CASE WHEN target_id = ?1 THEN ?2 ELSE target_id END)",
                    params![dup.id, keeper],
                )
                .map_err(storage)?;
                conn.execute("DELETE FROM memories WHERE id = ?1", [&dup.id]).map_err(storage)?;
                drop(conn);
                self.cache_remove(&dup.id);
                report.merged_duplicates += 1;
            }
        }

        // 3. Relaciones inferidas débiles.
        report.pruned_relations = self
            .database()
            .conn()
            .execute(
                "DELETE FROM memory_relations WHERE provenance = ?1 AND verified = 0 AND confidence < ?2",
                params![Provenance::Inferred.as_str(), PRUNE_BELOW],
            )
            .map_err(storage)?;

        let detail = serde_json::to_string(&report).unwrap_or_default();
        self.repo.log_event("MEMORY_CONSOLIDATED", None, &[], Some(&detail))?;
        self.emit(VisualEvent::MemoryConsolidated {
            removed: report.removed_expired,
            merged: report.merged_duplicates,
            pruned_relations: report.pruned_relations,
        });
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MemoryKind, MemoryRelation, NewMemory, RelationKind, RetentionPolicy};

    #[tokio::test]
    async fn applies_retention_merges_duplicates_and_prunes_weak_inferences() {
        let e = MemoryEngine::for_tests().expect("e");
        let mut temp = NewMemory::user(MemoryKind::Semantic, "temporal de esta sesión");
        temp.retention = RetentionPolicy::Session;
        e.remember(temp).await.expect("t");

        let keep = e.remember(NewMemory::user(MemoryKind::Semantic, "alfa beta")).await.expect("k").memory;
        let other = e.remember(NewMemory::user(MemoryKind::Semantic, "gamma delta")).await.expect("o").memory;
        // Un duplicado que entró sin pasar por `remember` (p. ej. una base antigua o una importación).
        let mut dup = keep.clone();
        dup.id = "dup-1".into();
        dup.verification = Verification::Unverified;
        dup.created_at = Utc::now();
        e.repository().insert(&dup).expect("dup");
        e.relate("dup-1", &other.id, RelationKind::DependsOn, Provenance::User, None).expect("rel");
        // Inferida débil.
        e.repository()
            .insert_relation(&MemoryRelation {
                id: "weak".into(),
                source_id: keep.id.clone(),
                target_id: other.id.clone(),
                kind: RelationKind::RelatedTo,
                confidence: 0.2,
                provenance: Provenance::Inferred,
                verified: false,
                note: None,
                created_at: Utc::now(),
            })
            .expect("weak");

        let r = e.consolidate().expect("c");
        assert_eq!(r.removed_expired, 1);
        assert_eq!(r.merged_duplicates, 1);
        assert_eq!(r.pruned_relations, 1);
        assert!(e.repository().get("dup-1").expect("g").is_none());
        let rels = e.repository().relations_of(&keep.id).expect("rels");
        assert_eq!(rels.len(), 1, "la relación del duplicado pasó al que se quedó");
        assert_eq!(rels[0].kind, RelationKind::DependsOn);
        assert_eq!(e.repository().count().expect("n"), 2);
    }
}
