use serde::Serialize;
use serde_json::Value;

use super::tool::{RiskClass, VerificationStatus};

/// Fases del ciclo OBSERVE → UNDERSTAND → PLAN → AUTHORIZE → EXECUTE → VERIFY → REMEMBER.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Observe,
    Understand,
    Plan,
    Authorize,
    Execute,
    Verify,
    Remember,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorCode {
    LlmUnavailable,
    ModelNotFound,
    LlmError,
    Timeout,
    Internal,
}

/// Evento que el orquestador emite hacia la UI. Contrato espejo en
/// `packages/shared-types/src/agent.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum AgentEvent {
    RunStarted { run_id: String, conversation_id: String },
    Phase { run_id: String, phase: Phase },
    Token { run_id: String, text: String },
    ToolCall { run_id: String, call_id: String, tool: String, arguments: Value, risk: RiskClass },
    ApprovalRequired {
        run_id: String,
        approval_id: String,
        call_id: String,
        tool: String,
        arguments: Value,
        risk: RiskClass,
        reason: String,
    },
    ApprovalResolved { run_id: String, approval_id: String, approved: bool, reason: Option<String> },
    ToolResult {
        run_id: String,
        call_id: String,
        tool: String,
        ok: bool,
        output: Value,
        error: Option<String>,
        verification: VerificationStatus,
        duration_ms: u64,
    },
    Final { run_id: String, text: String },
    Error { run_id: String, code: AgentErrorCode, message: String },
    Cancelled { run_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_serialize_with_the_ui_contract() {
        let e = AgentEvent::ToolResult {
            run_id: "r".into(),
            call_id: "c".into(),
            tool: "get_current_time".into(),
            ok: true,
            output: json!({"a": 1}),
            error: None,
            verification: VerificationStatus::Passed,
            duration_ms: 3,
        };
        let v = serde_json::to_value(&e).expect("json");
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["runId"], "r");
        assert_eq!(v["callId"], "c");
        assert_eq!(v["durationMs"], 3);
        assert_eq!(v["verification"], "passed");

        let p = serde_json::to_value(AgentEvent::Phase { run_id: "r".into(), phase: Phase::Authorize }).expect("json");
        assert_eq!(p, json!({"type": "phase", "runId": "r", "phase": "authorize"}));
    }
}

/// Eventos para la visualización (se emiten a toda la UI con `app.emit("jarvis://visual", …)`).
/// Espejo en `packages/shared-types/src/visual.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE", rename_all_fields = "camelCase")]
pub enum VisualEvent {
    MemoryCreated { node: super::graph::GraphNode, edges: Vec<super::graph::GraphEdge> },
    MemoryRetrieved { run_id: Option<String>, memory_ids: Vec<String>, relation_ids: Vec<String> },
    /// Recuerdos que solo estaban en el Cloud Core y se usaron en esta instrucción (se muestran como nodos de la nube).
    MemoryRetrievedRemote {
        run_id: Option<String>,
        nodes: Vec<super::graph::GraphNode>,
        edges: Vec<super::graph::GraphEdge>,
        relation_ids: Vec<String>,
    },
    MemoryUpdated { node: super::graph::GraphNode },
    MemoryDeleted { memory_id: String },
    MemoryConnected { edge: super::graph::GraphEdge },
    MemoryConsolidated { removed: usize, merged: usize, pruned_relations: usize },
    /// Importación o restauración: la UI recarga el grafo.
    MemoryImported { imported: usize, restored: bool },
    SkillLearned { skill_id: String, name: String },
    TaskStarted { run_id: String, goal: String },
    TaskCompleted { run_id: String, status: String },
}

#[cfg(test)]
mod visual_tests {
    use super::VisualEvent;

    #[test]
    fn visual_events_use_screaming_names_and_camel_fields() {
        let v = serde_json::to_value(VisualEvent::MemoryRetrieved {
            run_id: Some("r".into()),
            memory_ids: vec!["m".into()],
            relation_ids: vec![],
        })
        .expect("json");
        assert_eq!(v["type"], "MEMORY_RETRIEVED");
        assert_eq!(v["memoryIds"][0], "m");
        let t = serde_json::to_value(VisualEvent::TaskCompleted { run_id: "r".into(), status: "completed".into() }).expect("json");
        assert_eq!(t["type"], "TASK_COMPLETED");
    }
}
