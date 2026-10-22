// Eventos del ciclo del agente. Espejo de `domain::events::AgentEvent` en Rust
// (serde: tag = "type", variantes snake_case, campos camelCase).

export type AgentPhase =
  | "observe"
  | "understand"
  | "plan"
  | "authorize"
  | "execute"
  | "verify"
  | "remember";

export type RiskClass = "READ_ONLY" | "LOW_RISK" | "SENSITIVE" | "DESTRUCTIVE";

export type VerificationStatus = "passed" | "failed" | "inconclusive" | "not_applicable";

export type AgentErrorCode =
  | "llm_unavailable"
  | "model_not_found"
  | "llm_error"
  | "timeout"
  | "internal";

export type AgentEvent =
  | { type: "run_started"; runId: string; conversationId: string }
  | { type: "phase"; runId: string; phase: AgentPhase }
  | { type: "token"; runId: string; text: string }
  | {
      type: "tool_call";
      runId: string;
      callId: string;
      tool: string;
      arguments: unknown;
      risk: RiskClass;
    }
  | {
      type: "approval_required";
      runId: string;
      approvalId: string;
      callId: string;
      tool: string;
      arguments: unknown;
      risk: RiskClass;
      reason: string;
    }
  | {
      type: "approval_resolved";
      runId: string;
      approvalId: string;
      approved: boolean;
      reason?: string | null;
    }
  | {
      type: "tool_result";
      runId: string;
      callId: string;
      tool: string;
      ok: boolean;
      output: unknown;
      error?: string | null;
      verification: VerificationStatus;
      durationMs: number;
    }
  | { type: "final"; runId: string; text: string }
  | { type: "error"; runId: string; code: AgentErrorCode; message: string }
  | { type: "cancelled"; runId: string };

export interface RunInfo {
  runId: string;
  conversationId: string;
}
