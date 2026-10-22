import { describe, expect, it } from "vitest";
import type { AgentEvent } from "@jarvis/shared-types";
import { applyAgentEvent, initialAssistantState, type AssistantState } from "./assistantStore";

const run = (events: AgentEvent[], start: AssistantState = initialAssistantState) => events.reduce(applyAgentEvent, start);

describe("applyAgentEvent", () => {
  it("acumula tokens y los sustituye por la respuesta final", () => {
    const s = run([
      { type: "run_started", runId: "r", conversationId: "c" },
      { type: "phase", runId: "r", phase: "plan" },
      { type: "token", runId: "r", text: "Ho" },
      { type: "token", runId: "r", text: "la" },
    ]);
    expect(s.messages[0].text).toBe("Hola");
    expect(s.messages[0].streaming).toBe(true);
    expect(s.activeRun?.phase).toBe("plan");

    const done = applyAgentEvent(s, { type: "final", runId: "r", text: "Hola, Anderson." });
    expect(done.messages[0].text).toBe("Hola, Anderson.");
    expect(done.messages[0].streaming).toBe(false);
    expect(done.activeRun).toBeNull();
    expect(done.lastPhases).toEqual(["plan"]);
    expect(done.conversationId).toBe("c");
  });

  it("sigue el ciclo de aprobación de una herramienta", () => {
    let s = run([
      { type: "run_started", runId: "r", conversationId: "c" },
      { type: "tool_call", runId: "r", callId: "k", tool: "close_app", arguments: { appId: "notepad" }, risk: "SENSITIVE" },
      { type: "approval_required", runId: "r", approvalId: "a", callId: "k", tool: "close_app", arguments: {}, risk: "SENSITIVE", reason: "riesgo" },
    ]);
    expect(s.messages[0].steps[0]).toMatchObject({ status: "awaiting", approvalId: "a" });

    s = applyAgentEvent(s, { type: "approval_resolved", runId: "r", approvalId: "a", approved: true });
    expect(s.messages[0].steps[0].status).toBe("running");

    s = applyAgentEvent(s, {
      type: "tool_result", runId: "r", callId: "k", tool: "close_app", ok: true, output: { closed: true }, verification: "passed", durationMs: 12,
    });
    expect(s.messages[0].steps[0]).toMatchObject({ status: "done", verification: "passed", durationMs: 12 });
  });

  it("una aprobación rechazada queda como no autorizada aunque llegue el resultado", () => {
    const s = run([
      { type: "tool_call", runId: "r", callId: "k", tool: "wipe", arguments: {}, risk: "DESTRUCTIVE" },
      { type: "approval_required", runId: "r", approvalId: "a", callId: "k", tool: "wipe", arguments: {}, risk: "DESTRUCTIVE", reason: "x" },
      { type: "approval_resolved", runId: "r", approvalId: "a", approved: false, reason: "el usuario la rechazó" },
      { type: "tool_result", runId: "r", callId: "k", tool: "wipe", ok: false, output: null, error: "no", verification: "not_applicable", durationMs: 0 },
    ]);
    expect(s.messages[0].steps[0]).toMatchObject({ status: "denied", reason: "el usuario la rechazó" });
  });

  it("muestra errores del modelo y cancelaciones", () => {
    const err = run([
      { type: "run_started", runId: "r", conversationId: "c" },
      { type: "error", runId: "r", code: "model_not_found", message: "ollama pull qwen3:4b" },
    ]);
    expect(err.messages[0].error?.code).toBe("model_not_found");
    expect(err.activeRun).toBeNull();

    const cancelled = run([
      { type: "run_started", runId: "r2", conversationId: "c" },
      { type: "tool_call", runId: "r2", callId: "k", tool: "x", arguments: {}, risk: "LOW_RISK" },
      { type: "cancelled", runId: "r2" },
    ]);
    expect(cancelled.messages[0].cancelled).toBe(true);
    expect(cancelled.messages[0].steps[0].status).toBe("denied");
  });
});
