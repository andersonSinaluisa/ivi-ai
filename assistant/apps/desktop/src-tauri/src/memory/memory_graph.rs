//! Grafo de conocimiento: relaciones tipadas con procedencia y confianza, vecinos, recorridos
//! acotados y subgrafos para la visualización.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::Utc;

use super::embedding_provider::cosine;
use super::memory_engine::MemoryEngine;
use super::{MemoryError, MemoryResult};
use crate::domain::{
    GraphEdge, GraphNode, GraphView, MemoryRecord, MemoryRelation, Provenance, RelationKind, VisualEvent,
};

/// Límite duro de nodos en una vista (el renderizado no debe recibir miles de nodos).
pub const MAX_VIEW_NODES: usize = 1_500;

impl MemoryEngine {
    /// Crea la relación si no existe. Devuelve `(relación, creada)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn relate_internal(
        &self,
        source_id: &str,
        target_id: &str,
        kind: RelationKind,
        provenance: Provenance,
        confidence: f32,
        verified: bool,
        note: Option<String>,
    ) -> MemoryResult<(MemoryRelation, bool)> {
        if source_id == target_id {
            return Err(MemoryError::Invalid("un recuerdo no puede relacionarse consigo mismo".into()));
        }
        if self.repo.get(source_id)?.is_none() || self.repo.get(target_id)?.is_none() {
            return Err(MemoryError::NotFound);
        }
        let relation = MemoryRelation {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source_id.to_owned(),
            target_id: target_id.to_owned(),
            kind,
            confidence: confidence.clamp(0.0, 1.0),
            provenance,
            // Solo el usuario puede afirmar una relación como verificada.
            verified: verified && provenance == Provenance::User,
            note,
            created_at: Utc::now(),
        };
        let created = self.repo.insert_relation(&relation)?;
        Ok((relation, created))
    }

    /// Relación creada desde la UI o por el agente.
    ///
    /// # Errors
    /// Extremos inexistentes o iguales.
    pub fn relate(
        &self,
        source_id: &str,
        target_id: &str,
        kind: RelationKind,
        provenance: Provenance,
        note: Option<String>,
    ) -> MemoryResult<MemoryRelation> {
        let confidence = if provenance == Provenance::User { 1.0 } else { 0.7 };
        let (relation, created) =
            self.relate_internal(source_id, target_id, kind, provenance, confidence, provenance == Provenance::User, note)?;
        if created {
            self.repo.log_event("MEMORY_CONNECTED", None, &[source_id.to_owned(), target_id.to_owned()], Some(kind.as_str()))?;
            self.emit(VisualEvent::MemoryConnected { edge: GraphEdge::from(&relation) });
        }
        Ok(relation)
    }

    /// El usuario confirma (o retira) una relación.
    ///
    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn set_relation_verified(&self, id: &str, verified: bool) -> MemoryResult<MemoryRelation> {
        if !self.repo.set_relation_verified(id, verified)? {
            return Err(MemoryError::NotFound);
        }
        let r = self.repo.get_relation(id)?.ok_or(MemoryError::NotFound)?;
        self.emit(VisualEvent::MemoryConnected { edge: GraphEdge::from(&r) });
        Ok(r)
    }

    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn delete_relation(&self, id: &str) -> MemoryResult<()> {
        if self.repo.delete_relation(id)? {
            Ok(())
        } else {
            Err(MemoryError::NotFound)
        }
    }

    /// Relaciones inferidas por similitud con los recuerdos más parecidos.
    pub(crate) fn autolink(&self, id: &str, vector: &[f32]) -> MemoryResult<Vec<MemoryRelation>> {
        let threshold =
            if self.embedder().is_semantic() { self.config.autolink_semantic } else { self.config.autolink_lexical };
        let mut scored: Vec<(String, f32)> = self
            .vectors()?
            .into_iter()
            .filter(|(other, _)| other != id)
            .map(|(other, v)| {
                let s = cosine(vector, &v);
                (other, s)
            })
            .filter(|(_, s)| *s >= threshold)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut out = Vec::new();
        for (other, sim) in scored.into_iter().take(self.config.autolink_max) {
            let (r, created) = self.relate_internal(id, &other, RelationKind::RelatedTo, Provenance::Inferred, sim, false, Some(format!("similitud {sim:.2}")))?;
            if created {
                out.push(r);
            }
        }
        Ok(out)
    }

    /// Vecinos directos con su relación.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub fn neighbors(&self, id: &str) -> MemoryResult<Vec<(MemoryRelation, MemoryRecord)>> {
        let rels = self.repo.relations_of(id)?;
        let ids: Vec<String> =
            rels.iter().map(|r| if r.source_id == id { r.target_id.clone() } else { r.source_id.clone() }).collect();
        let mems: HashMap<String, MemoryRecord> = self.repo.get_many(&ids)?.into_iter().map(|m| (m.id.clone(), m)).collect();
        Ok(rels
            .into_iter()
            .zip(ids)
            .filter_map(|(r, other)| mems.get(&other).cloned().map(|m| (r, m)))
            .collect())
    }

    /// Recorrido en anchura limitado por profundidad, número de nodos y confianza mínima de las aristas.
    /// Devuelve `(id, profundidad)` en orden de visita.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub fn traverse(&self, start: &str, max_depth: usize, max_nodes: usize, min_confidence: f32) -> MemoryResult<Vec<(String, usize)>> {
        let mut seen: HashSet<String> = HashSet::from([start.to_owned()]);
        let mut out = vec![(start.to_owned(), 0)];
        let mut queue = VecDeque::from([(start.to_owned(), 0usize)]);
        while let Some((id, depth)) = queue.pop_front() {
            if depth >= max_depth || out.len() >= max_nodes {
                continue;
            }
            for r in self.repo.relations_of(&id)? {
                if r.confidence < min_confidence {
                    continue;
                }
                let next = if r.source_id == id { r.target_id } else { r.source_id };
                if seen.insert(next.clone()) {
                    out.push((next.clone(), depth + 1));
                    if out.len() >= max_nodes {
                        break;
                    }
                    queue.push_back((next, depth + 1));
                }
            }
        }
        Ok(out)
    }

    /// Subgrafo inducido por un conjunto de recuerdos.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub fn subgraph(&self, ids: &[String]) -> MemoryResult<GraphView> {
        let ids: Vec<String> = ids.iter().take(MAX_VIEW_NODES).cloned().collect();
        let set: HashSet<String> = ids.iter().cloned().collect();
        let mems = self.repo.get_many(&ids)?;
        let degrees = self.repo.degrees(&ids)?;
        let edges = self.repo.relations_among(&set)?;
        Ok(GraphView {
            nodes: mems.iter().map(|m| GraphNode::from_record(m, degrees.get(&m.id).copied().unwrap_or(0))).collect(),
            edges: edges.iter().map(GraphEdge::from).collect(),
            total_memories: self.repo.count()?,
            truncated: false,
        })
    }

    /// Vista general paginada: los recuerdos más recientes y las relaciones entre ellos.
    ///
    /// # Errors
    /// Errores de almacenamiento.
    pub fn graph_overview(&self, limit: usize, offset: usize) -> MemoryResult<GraphView> {
        let limit = limit.clamp(1, MAX_VIEW_NODES);
        let ids: Vec<String> = self.repo.recent(limit, offset)?.into_iter().map(|m| m.id).collect();
        let mut view = self.subgraph(&ids)?;
        view.truncated = view.total_memories > (offset + view.nodes.len()) as u64;
        Ok(view)
    }

    /// Vista centrada en un recuerdo, hasta `depth` saltos.
    ///
    /// # Errors
    /// [`MemoryError::NotFound`].
    pub fn graph_focus(&self, id: &str, depth: usize, limit: usize) -> MemoryResult<GraphView> {
        if self.repo.get(id)?.is_none() {
            return Err(MemoryError::NotFound);
        }
        let limit = limit.clamp(1, MAX_VIEW_NODES);
        let visited = self.traverse(id, depth.min(4), limit, 0.0)?;
        let ids: Vec<String> = visited.into_iter().map(|(i, _)| i).collect();
        let mut view = self.subgraph(&ids)?;
        view.truncated = ids.len() >= limit;
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MemoryKind, NewMemory};

    async fn add(e: &MemoryEngine, text: &str) -> String {
        e.remember(NewMemory::user(MemoryKind::Semantic, text)).await.expect("add").memory.id
    }

    /// Cadena a → b → c → d sin relaciones inferidas (textos sin palabras en común).
    async fn chain(e: &MemoryEngine) -> Vec<String> {
        let ids = vec![
            add(e, "alfa uno").await,
            add(e, "bravo dos").await,
            add(e, "charlie tres").await,
            add(e, "delta cuatro").await,
        ];
        for w in ids.windows(2) {
            e.relate(&w[0], &w[1], RelationKind::DependsOn, Provenance::User, None).expect("rel");
        }
        ids
    }

    #[tokio::test]
    async fn relations_are_typed_with_provenance_and_unique() {
        let e = MemoryEngine::for_tests().expect("e");
        let ids = chain(&e).await;
        let r = e.relate(&ids[0], &ids[1], RelationKind::DependsOn, Provenance::User, None).expect("again");
        assert_eq!(e.repository().relations_of(&ids[0]).expect("r").len(), 1, "no se duplica");
        assert!(r.verified);
        let agent = e.relate(&ids[0], &ids[2], RelationKind::CausedBy, Provenance::Agent, None).expect("agent");
        assert!(!agent.verified);
        assert!((agent.confidence - 0.7).abs() < f32::EPSILON);
        assert!(e.relate(&ids[0], &ids[0], RelationKind::RelatedTo, Provenance::User, None).is_err());
        assert!(matches!(e.relate(&ids[0], "nope", RelationKind::RelatedTo, Provenance::User, None), Err(MemoryError::NotFound)));
        let verified = e.set_relation_verified(&agent.id, true).expect("verify");
        assert!(verified.verified);
    }

    #[tokio::test]
    async fn neighbors_and_bounded_traversal() {
        let e = MemoryEngine::for_tests().expect("e");
        let ids = chain(&e).await;
        assert_eq!(e.neighbors(&ids[1]).expect("n").len(), 2);
        let two = e.traverse(&ids[0], 2, 100, 0.0).expect("t");
        assert_eq!(two.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>(), ids[..3].to_vec());
        assert_eq!(two[2].1, 2);
        assert_eq!(e.traverse(&ids[0], 10, 2, 0.0).expect("t").len(), 2, "límite de nodos");
    }

    #[tokio::test]
    async fn subgraphs_overview_and_focus() {
        let e = MemoryEngine::for_tests().expect("e");
        let ids = chain(&e).await;
        let sub = e.subgraph(&ids[..2]).expect("s");
        assert_eq!(sub.nodes.len(), 2);
        assert_eq!(sub.edges.len(), 1);
        let overview = e.graph_overview(3, 0).expect("o");
        assert_eq!(overview.nodes.len(), 3);
        assert!(overview.truncated);
        assert_eq!(overview.total_memories, 4);
        let focus = e.graph_focus(&ids[3], 1, 50).expect("f");
        assert_eq!(focus.nodes.len(), 2);
        let d = focus.nodes.iter().find(|n| n.id == ids[2]).expect("c");
        assert_eq!(d.degree, 2);
    }
}
