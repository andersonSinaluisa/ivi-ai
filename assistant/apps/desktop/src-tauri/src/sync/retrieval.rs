//! Recuperación híbrida: primero la memoria local (siempre, con o sin Internet); en modo híbrido se completa con
//! una búsqueda en el Cloud Core (timeout corto). Lo que solo está en el Cloud se añade al contexto, se muestra en
//! el Neural Core como nodos de la nube y se pide un pull para traerlo.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use super::engine::{SyncEngine, SyncMode};
use super::protocol::SearchResponse;
use crate::domain::graph::{GraphEdge, GraphNode};
use crate::domain::memory::{MemoryKind, MemoryRecord, Provenance, RelationKind, RetentionPolicy, Verification};
use crate::domain::VisualEvent;
use crate::memory::memory_retriever::{build_context, RetrievalOptions, RetrievalResult, RetrievedMemory, Via};
use crate::memory::{MemoryEngine, MemoryResult, MemoryRetriever};

pub struct HybridRetriever {
    memory: Arc<MemoryEngine>,
    sync: Arc<SyncEngine>,
}

impl HybridRetriever {
    #[must_use]
    pub fn new(memory: Arc<MemoryEngine>, sync: Arc<SyncEngine>) -> Self {
        Self { memory, sync }
    }
}

/// Recuerdos devueltos por el Cloud que no están en esta PC, como resultados de recuperación.
#[must_use]
pub fn remote_only(response: &SearchResponse, local: &HashSet<String>, exists_locally: impl Fn(&str) -> bool) -> Vec<RetrievedMemory> {
    response
        .memories
        .iter()
        .filter(|h| !local.contains(&h.id) && !exists_locally(&h.id))
        .filter(|h| h.verification != "disputed")
        .map(|h| RetrievedMemory {
            memory: MemoryRecord {
                id: h.id.clone(),
                kind: MemoryKind::parse(&h.kind).unwrap_or(MemoryKind::Semantic),
                content: h.content.clone(),
                summary: h.summary.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                source: "Cloud Core".into(),
                source_id: None,
                relevance: 0.5,
                confidence: h.confidence,
                verification: Verification::parse(&h.verification).unwrap_or_default(),
                tags: h.tags.clone(),
                references: Vec::new(),
                retention: RetentionPolicy::Permanent,
                content_hash: String::new(),
            },
            #[allow(clippy::cast_possible_truncation)]
            score: h.score as f32,
            #[allow(clippy::cast_possible_truncation)]
            similarity: h.score as f32,
            via: Via::Direct,
        })
        .collect()
}

#[async_trait]
impl MemoryRetriever for HybridRetriever {
    async fn retrieve_context(&self, query: &str, options: RetrievalOptions) -> MemoryResult<RetrievalResult> {
        let mut local = self.memory.retrieve(query, &options).await?;
        if self.sync.status().mode != SyncMode::Hybrid {
            return Ok(local);
        }

        let embedder = self.memory.embedder();
        let embedding = if embedder.is_semantic() {
            embedder.embed(&[query.to_owned()]).await.ok().and_then(|mut v| v.pop()).map(|v| (embedder.model_id(), v))
        } else {
            None
        };
        let limit = u32::try_from(options.limit.max(1)).unwrap_or(8);
        let Some(response) = self.sync.remote_search(query, embedding, limit).await else {
            return Ok(local);
        };

        let ids: HashSet<String> = local.items.iter().map(|i| i.memory.id.clone()).collect();
        let repo = self.memory.repository();
        let extra = remote_only(&response, &ids, |id| repo.get(id).ok().flatten().is_some());
        if extra.is_empty() {
            return Ok(local);
        }

        let room = options.limit.saturating_sub(local.items.len()).max(2);
        let extra: Vec<RetrievedMemory> = extra.into_iter().take(room).collect();
        let remote_ids: HashSet<String> = extra.iter().map(|e| e.memory.id.clone()).collect();
        let nodes: Vec<GraphNode> = extra.iter().map(|e| GraphNode::from_record(&e.memory, 0)).collect();
        let edges: Vec<GraphEdge> = response
            .relations
            .iter()
            .filter(|r| remote_ids.contains(&r.source_id) || remote_ids.contains(&r.target_id))
            .map(|r| GraphEdge {
                id: r.id.clone(),
                source: r.source_id.clone(),
                target: r.target_id.clone(),
                kind: RelationKind::parse(&r.kind).unwrap_or(RelationKind::RelatedTo),
                confidence: r.confidence,
                provenance: Provenance::parse(&r.provenance).unwrap_or(Provenance::Import),
                verified: r.verified,
            })
            .collect();
        self.memory.emit(VisualEvent::MemoryRetrievedRemote {
            run_id: options.run_id.clone(),
            nodes,
            relation_ids: edges.iter().map(|e| e.id.clone()).collect(),
            edges,
        });

        local.items.extend(extra);
        let (context, tokens, _) = build_context(&local.items, options.token_budget);
        local.context = context;
        local.tokens_estimate = tokens;
        // Lo que solo está en la nube se trae en el próximo ciclo.
        self.sync.nudge();
        Ok(local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::protocol::SearchHit;

    #[test]
    fn only_memories_missing_here_are_added_and_disputed_ones_never() {
        let hit = |id: &str, verification: &str| SearchHit {
            id: id.into(),
            version: 1,
            kind: "semantic".into(),
            content: format!("contenido {id}"),
            summary: String::new(),
            tags: vec![],
            confidence: 0.9,
            verification: verification.into(),
            score: 0.8,
            via: "match".into(),
        };
        let response = SearchResponse {
            method: "text".into(),
            memories: vec![hit("ya-recuperado", "verified"), hit("ya-guardado", "verified"), hit("solo-nube", "verified"), hit("dudoso", "disputed")],
            relations: vec![],
        };
        let local: HashSet<String> = ["ya-recuperado".to_owned()].into();
        let extra = remote_only(&response, &local, |id| id == "ya-guardado");
        assert_eq!(extra.iter().map(|e| e.memory.id.as_str()).collect::<Vec<_>>(), vec!["solo-nube"]);
        assert_eq!(extra[0].memory.source, "Cloud Core");
    }
}
