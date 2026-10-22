// Grafo de conocimiento y eventos visuales. Espejo de domain::graph y domain::events::VisualEvent.

export type MemoryKind = "episodic" | "semantic" | "procedural" | "experience" | "entity";
export type Verification = "unverified" | "verified" | "disputed";
export type RelationKind =
  | "BELONGS_TO"
  | "DEPENDS_ON"
  | "CAUSED_BY"
  | "SOLVED_BY"
  | "RELATED_TO"
  | "LEARNED_FROM"
  | "USED_IN"
  | "DUPLICATE_OF";
export type Provenance = "user" | "agent" | "inferred" | "import" | "consolidation";

export interface GraphNode {
  id: string;
  kind: MemoryKind;
  label: string;
  confidence: number;
  verification: Verification;
  degree: number;
  createdAt: string;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  kind: RelationKind;
  confidence: number;
  provenance: Provenance;
  verified: boolean;
}

export interface GraphView {
  nodes: GraphNode[];
  edges: GraphEdge[];
  totalMemories: number;
  truncated: boolean;
}

/** Eventos emitidos por Rust en "jarvis://visual". */
export type BackendVisualEvent =
  | { type: "MEMORY_CREATED"; node: GraphNode; edges: GraphEdge[] }
  | { type: "MEMORY_RETRIEVED"; runId: string | null; memoryIds: string[]; relationIds: string[] }
  /** Recuerdos que solo estaban en el Cloud Core y se usaron en esta instrucción. */
  | { type: "MEMORY_RETRIEVED_REMOTE"; runId: string | null; nodes: GraphNode[]; edges: GraphEdge[]; relationIds: string[] }
  | { type: "MEMORY_UPDATED"; node: GraphNode }
  | { type: "MEMORY_DELETED"; memoryId: string }
  | { type: "MEMORY_CONNECTED"; edge: GraphEdge }
  | { type: "MEMORY_CONSOLIDATED"; removed: number; merged: number; prunedRelations: number }
  | { type: "MEMORY_IMPORTED"; imported: number; restored: boolean }
  | { type: "SKILL_LEARNED"; skillId: string; name: string }
  | { type: "TASK_STARTED"; runId: string; goal: string }
  | { type: "TASK_COMPLETED"; runId: string; status: string };

/** Eventos de voz que la UI deriva del servicio de voz. */
export type VoiceVisualEvent =
  | { type: "VOICE_LISTENING"; active: boolean }
  | { type: "VOICE_SPEAKING"; active: boolean; amplitude: number };

export type VisualEvent = BackendVisualEvent | VoiceVisualEvent;
