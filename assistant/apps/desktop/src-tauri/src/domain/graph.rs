//! Vista del grafo de conocimiento para la interfaz.

use serde::Serialize;

use super::memory::{MemoryKind, MemoryRecord, MemoryRelation, Provenance, RelationKind, Verification};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub kind: MemoryKind,
    pub label: String,
    pub confidence: f32,
    pub verification: Verification,
    pub degree: u32,
    pub created_at: String,
}

impl GraphNode {
    #[must_use]
    pub fn from_record(m: &MemoryRecord, degree: u32) -> Self {
        Self {
            id: m.id.clone(),
            kind: m.kind,
            label: m.label(),
            confidence: m.confidence,
            verification: m.verification,
            degree,
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: RelationKind,
    pub confidence: f32,
    pub provenance: Provenance,
    pub verified: bool,
}

impl From<&MemoryRelation> for GraphEdge {
    fn from(r: &MemoryRelation) -> Self {
        Self {
            id: r.id.clone(),
            source: r.source_id.clone(),
            target: r.target_id.clone(),
            kind: r.kind,
            confidence: r.confidence,
            provenance: r.provenance,
            verified: r.verified,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Total de recuerdos en la base (la vista puede mostrar solo una parte).
    pub total_memories: u64,
    pub truncated: bool,
}
