//! Tipos del dominio. No dependen de Tauri, de Ollama ni de SQLite.

pub mod chat;
pub mod events;
pub mod experience;
pub mod graph;
pub mod memory;
pub mod tool;

pub use chat::{ChatMessage, Role, ToolCall};
pub use events::{AgentErrorCode, AgentEvent, Phase, VisualEvent};
pub use experience::{Experience, ExperienceStep};
pub use graph::{GraphEdge, GraphNode, GraphView};
pub use memory::{
    MemoryKind, MemoryPatch, MemoryRecord, MemoryRelation, MemorySource, NewMemory, NewRelation, NewSource, Provenance,
    RelationKind, RetentionPolicy, Skill, SkillVersion, SourceKind, Verification,
};
pub use tool::{RiskClass, ToolOutput, ToolSource, ToolSpec, VerificationStatus};
