//! Crear, editar, consultar y borrar recuerdos.

use chrono::Utc;
use serde::Serialize;

use super::memory_engine::MemoryEngine;
use super::memory_repository::{content_hash, MemoryEventRow, MemoryFilter, MemoryPage};
use super::secrets::contains_secret;
use super::{MemoryError, MemoryResult};
use crate::domain::{
    GraphEdge, GraphNode, MemoryPatch, MemoryRecord, MemoryRelation, MemorySource, NewMemory, Provenance, SourceKind,
    VisualEvent,
};

pub const MAX_CONTENT: usize = 8_000;
const MAX_TAGS: usize = 20;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberOutcome {
    pub memory: MemoryRecord,
    /// `false` si ya existía un recuerdo con el mismo contenido.
    pub created: bool,
    pub relations: Vec<MemoryRelation>,
    /// El vector no se pudo calcular ahora (se reintenta con `reembed`).
    pub embedding_pending: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationView {
    pub relation: MemoryRelation,
    pub other: GraphNode,
    pub outgoing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDetail {
    pub memory: MemoryRecord,
    pub source: Option<MemorySource>,
    pub relations: Vec<RelationView>,
    pub events: Vec<MemoryEventRow>,
}

fn clean_tags(tags: &[String]) -> MemoryResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in tags.iter().take(MAX_TAGS) {
        let t = t.trim().trim_start_matches('#').to_lowercase();
        if t.is_empty() || t.chars().count() > 40 {
            continue;
        }
        if contains_secret(&t) {
            return Err(MemoryError::ContainsSecret);
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

fn validate_content(content: &str) -> MemoryResult<String> {
    let content = content.trim();
    if content.is_empty() {
        return Err(MemoryError::Invalid("el recuerdo está vacío".into()));
    }
    if content.chars().count() > MAX_CONTENT {
        return Err(MemoryError::Invalid(format!("máximo {MAX_CONTENT} caracteres")));
    }
    if contains_secret(content) {
        return Err(MemoryError::ContainsSecret);
    }
    Ok(content.to_owned())
}

fn default_summary(content: &str) -> String {
    let first = content.split(['.', '\n']).next().unwrap_or(content).trim();
    first.chars().take(120).collect()
}

impl MemoryEngine {
    /// Guarda un recuerdo: valida, evita duplicados, calcula su vector, crea las relaciones
    /// explícitas y propone relaciones inferidas (marcadas como no verificadas).
    ///
    /// # Errors
    /// Contenido inválido o con secretos; errores de almacenamiento.
    pub async fn remember(&self, new: NewMemory) -> MemoryResult<RememberOutcome> {
        let content = validate_content(&new.content)?;
        let summary = new.summary.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        if summary.as_deref().is_some_and(contains_secret) {
            return Err(MemoryError::ContainsSecret);
        }
        let tags = clean_tags(&new.tags)?;
        let hash = content_hash(&content);

        if let Some(existing) = self.repo.find_by_hash(&hash)? {
            return Ok(RememberOutcome { memory: existing, created: false, relations: Vec::new(), embedding_pending: false });
        }

        let source = self.repo.insert_source(&new.source)?;
        let now = Utc::now();
        let record = MemoryRecord {
            id: uuid::Uuid::new_v4().to_string(),
            kind: new.kind,
            summary: summary.unwrap_or_else(|| default_summary(&content)),
            content,
            created_at: now,
            updated_at: now,
            source: source.label.clone(),
            source_id: Some(source.id.clone()),
            relevance: 0.5,
            confidence: new.confidence.clamp(0.0, 1.0),
            verification: new.verification,
            tags,
            references: Vec::new(),
            retention: new.retention,
            content_hash: hash.clone(),
        };
        self.repo.insert(&record)?;

        let text = Self::embedding_text(&record.summary, &record.content, &record.tags);
        let vector = match self.embed_memory(&record.id, &hash, &text).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "recuerdo guardado sin vector; se calculará después");
                None
            }
        };

        // Relaciones explícitas: las afirma quien crea el recuerdo.
        let provenance = match new.source.kind {
            SourceKind::User => Provenance::User,
            SourceKind::Import => Provenance::Import,
            _ => Provenance::Agent,
        };
        let mut relations = Vec::new();
        for rel in &new.relations {
            match self.relate_internal(&record.id, &rel.target_id, rel.kind, provenance, record.confidence, provenance == Provenance::User, None) {
                Ok((r, true)) => relations.push(r),
                Ok((_, false)) => {}
                Err(e) => tracing::warn!(error = %e, target = %rel.target_id, "relación explícita descartada"),
            }
        }
        // Relaciones inferidas por similitud: nunca verificadas.
        if let Some(v) = &vector {
            relations.extend(self.autolink(&record.id, v)?);
        }

        self.repo.log_event("MEMORY_CREATED", None, std::slice::from_ref(&record.id), Some(&format!("fuente: {}", source.label)))?;
        let degree = u32::try_from(relations.len()).unwrap_or(u32::MAX);
        self.emit(VisualEvent::MemoryCreated {
            node: GraphNode::from_record(&record, degree),
            edges: relations.iter().map(GraphEdge::from).collect(),
        });
        Ok(RememberOutcome { memory: record, created: true, relations, embedding_pending: vector.is_none() })
    }

    /// # Errors
    /// [`MemoryError::NotFound`], contenido inválido o con secretos.
    pub async fn update_memory(&self, id: &str, patch: MemoryPatch) -> MemoryResult<MemoryRecord> {
        let mut m = self.repo.get(id)?.ok_or(MemoryError::NotFound)?;
        let mut content_changed = false;
        if let Some(c) = patch.content {
            let c = validate_content(&c)?;
            if c != m.content {
                m.content = c;
                m.content_hash = content_hash(&m.content);
                content_changed = true;
            }
        }
        if let Some(s) = patch.summary {
            if contains_secret(&s) {
                return Err(MemoryError::ContainsSecret);
            }
            m.summary = s.trim().chars().take(300).collect();
        }
        if let Some(t) = patch.tags {
            m.tags = clean_tags(&t)?;
        }
        if let Some(c) = patch.confidence {
            m.confidence = c.clamp(0.0, 1.0);
        }
        if let Some(v) = patch.verification {
            m.verification = v;
        }
        if let Some(r) = patch.retention {
            m.retention = r;
        }
        m.updated_at = Utc::now();
        self.repo.update(&m)?;
        if content_changed {
            self.cache_remove(&m.id);
            let text = Self::embedding_text(&m.summary, &m.content, &m.tags);
            if let Err(e) = self.embed_memory(&m.id, &m.content_hash, &text).await {
                tracing::warn!(error = %e, "vector pendiente tras editar");
            }
        }
        self.repo.log_event("MEMORY_UPDATED", None, &[m.id.clone()], None)?;
        let degree = self.repo.degrees(&[m.id.clone()])?.get(&m.id).copied().unwrap_or(0);
        self.emit(VisualEvent::MemoryUpdated { node: GraphNode::from_record(&m, degree) });
        Ok(m)
    }

    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn delete_memory(&self, id: &str) -> MemoryResult<()> {
        if !self.repo.delete(id)? {
            return Err(MemoryError::NotFound);
        }
        self.cache_remove(id);
        self.repo.log_event("MEMORY_DELETED", None, &[id.to_owned()], None)?;
        self.emit(VisualEvent::MemoryDeleted { memory_id: id.to_owned() });
        Ok(())
    }

    /// # Errors
    /// Errores de almacenamiento.
    pub fn list_memories(&self, filter: &MemoryFilter, offset: u64, limit: u64) -> MemoryResult<MemoryPage> {
        self.repo.list(filter, offset, limit)
    }

    /// Recuerdo con su fuente, relaciones (con el otro extremo) y eventos.
    ///
    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn memory_detail(&self, id: &str) -> MemoryResult<MemoryDetail> {
        let memory = self.repo.get(id)?.ok_or(MemoryError::NotFound)?;
        let source = match &memory.source_id {
            Some(sid) => self.repo.get_source(sid)?,
            None => None,
        };
        let rels = self.repo.relations_of(id)?;
        let other_ids: Vec<String> =
            rels.iter().map(|r| if r.source_id == id { r.target_id.clone() } else { r.source_id.clone() }).collect();
        let others = self.repo.get_many(&other_ids)?;
        let degrees = self.repo.degrees(&other_ids)?;
        let relations = rels
            .into_iter()
            .filter_map(|r| {
                let outgoing = r.source_id == id;
                let other_id = if outgoing { &r.target_id } else { &r.source_id };
                let other = others.iter().find(|m| &m.id == other_id)?;
                Some(RelationView {
                    other: GraphNode::from_record(other, degrees.get(other_id).copied().unwrap_or(0)),
                    relation: r,
                    outgoing,
                })
            })
            .collect();
        Ok(MemoryDetail { memory, source, relations, events: self.repo.events(Some(id), None, 30)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MemoryKind, NewRelation, NewSource, RelationKind, Verification};
    use crate::memory::MemoryEngine;

    pub(crate) fn note(content: &str) -> NewMemory {
        NewMemory::user(MemoryKind::Semantic, content)
    }

    #[tokio::test]
    async fn remember_persists_with_source_hash_and_embedding() {
        let e = MemoryEngine::for_tests().expect("engine");
        let mut events = e.subscribe();
        let out = e.remember(note("Mi proyecto LOOP usa FastAPI y React.")).await.expect("ok");
        assert!(out.created && !out.embedding_pending);
        let m = &out.memory;
        assert_eq!(m.summary, "Mi proyecto LOOP usa FastAPI y React");
        assert_eq!(m.verification, Verification::Verified);
        assert!(!m.content_hash.is_empty());
        let detail = e.memory_detail(&m.id).expect("detail");
        assert_eq!(detail.source.expect("source").kind, SourceKind::User);
        assert_eq!(detail.events[0].kind, "MEMORY_CREATED");
        assert_eq!(e.embedding_status().expect("s").pending, 0);
        assert!(matches!(events.try_recv().expect("evento"), VisualEvent::MemoryCreated { .. }));
    }

    #[tokio::test]
    async fn duplicates_are_detected_by_normalized_content() {
        let e = MemoryEngine::for_tests().expect("engine");
        let a = e.remember(note("Vivo en Guayaquil")).await.expect("a");
        let b = e.remember(note("  vivo   en GUAYAQUIL ")).await.expect("b");
        assert!(!b.created);
        assert_eq!(a.memory.id, b.memory.id);
        assert_eq!(e.repository().count().expect("n"), 1);
    }

    #[tokio::test]
    async fn secrets_and_empty_content_are_rejected() {
        let e = MemoryEngine::for_tests().expect("engine");
        assert!(matches!(e.remember(note("mi password: hunter22")).await, Err(MemoryError::ContainsSecret)));
        assert!(matches!(e.remember(note("   ")).await, Err(MemoryError::Invalid(_))));
        let mut n = note("algo normal");
        n.tags = vec!["token: abc".into()];
        assert!(matches!(e.remember(n).await, Err(MemoryError::ContainsSecret)));
    }

    #[tokio::test]
    async fn similar_memories_get_inferred_unverified_relations_and_explicit_ones_are_kept() {
        let e = MemoryEngine::for_tests().expect("engine");
        let loop_ = e.remember(note("El proyecto LOOP es un SaaS de cupones")).await.expect("a").memory;
        let stack = e.remember(note("El proyecto LOOP usa FastAPI en el backend de cupones")).await.expect("b");
        let inferred = stack.relations.iter().find(|r| r.target_id == loop_.id).expect("relación inferida");
        assert_eq!(inferred.provenance, Provenance::Inferred);
        assert!(!inferred.verified, "una relación inferida no es un hecho verificado");

        let mut explicit = note("FastAPI es un framework de Python");
        explicit.relations = vec![NewRelation { target_id: stack.memory.id.clone(), kind: RelationKind::UsedIn }];
        explicit.source = NewSource { kind: SourceKind::Agent, reference: Some("run-1".into()), label: None };
        let out = e.remember(explicit).await.expect("c");
        let used_in = out.relations.iter().find(|r| r.kind == RelationKind::UsedIn).expect("explícita");
        assert_eq!(used_in.provenance, Provenance::Agent);
        assert!(!used_in.verified, "lo que afirma el agente tampoco queda verificado");
    }

    #[tokio::test]
    async fn update_reembeds_and_delete_cascades() {
        let e = MemoryEngine::for_tests().expect("engine");
        let a = e.remember(note("Uso Windows 11")).await.expect("a").memory;
        let b = e.remember(note("Uso Windows 11 en la laptop del trabajo")).await.expect("b").memory;
        let patch = MemoryPatch { content: Some("Uso Windows 11 Pro".into()), verification: Some(Verification::Disputed), ..MemoryPatch::default() };
        let updated = e.update_memory(&a.id, patch).await.expect("upd");
        assert_eq!(updated.verification, Verification::Disputed);
        assert_ne!(updated.content_hash, a.content_hash);
        assert_eq!(e.embedding_status().expect("s").pending, 0, "se recalculó el vector");
        e.delete_memory(&a.id).expect("del");
        assert!(e.repository().relations_of(&b.id).expect("rels").iter().all(|r| r.source_id != a.id && r.target_id != a.id));
        assert!(matches!(e.delete_memory(&a.id), Err(MemoryError::NotFound)));
    }

    #[tokio::test]
    async fn pagination_and_filters() {
        let e = MemoryEngine::for_tests().expect("engine");
        for i in 0..25 {
            e.remember(note(&format!("Nota número {i} sobre el tema {}", i % 3))).await.expect("n");
        }
        let page = e.list_memories(&MemoryFilter::default(), 20, 10).expect("p");
        assert_eq!(page.total, 25);
        assert_eq!(page.items.len(), 5);
        let filtered = e.list_memories(&MemoryFilter { query: Some("tema 1".into()), ..MemoryFilter::default() }, 0, 50).expect("f");
        assert_eq!(filtered.total, 8);
    }
}
