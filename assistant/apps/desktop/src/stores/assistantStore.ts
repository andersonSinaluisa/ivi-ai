import { create } from "zustand";
import type {
  AgentErrorCode,
  AgentEvent,
  AgentPhase,
  RiskClass,
  StoredMessage,
  VerificationStatus,
} from "@jarvis/shared-types";
import { backend } from "../services/backend";

export type StepStatus = "pending" | "awaiting" | "running" | "done" | "failed" | "denied";

export interface StepView {
  callId: string;
  tool: string;
  arguments: unknown;
  risk: RiskClass;
  status: StepStatus;
  approvalId?: string;
  reason?: string | null;
  output?: unknown;
  error?: string | null;
  verification?: VerificationStatus;
  durationMs?: number;
}

export interface UiMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  runId?: string;
  streaming?: boolean;
  cancelled?: boolean;
  error?: { code: AgentErrorCode; message: string };
  steps: StepView[];
  /** La instrucción llegó por voz: la respuesta se leerá en voz alta. */
  spoken?: boolean;
  spokenHandled?: boolean;
}

export interface ActiveRun {
  runId: string;
  phase: AgentPhase | null;
  phasesSeen: AgentPhase[];
}

export interface AssistantState {
  conversationId: string | null;
  messages: UiMessage[];
  activeRun: ActiveRun | null;
  /** Fases de la última ejecución terminada (para el núcleo del dashboard). */
  lastPhases: AgentPhase[];
  sendError: string | null;
}

export const initialAssistantState: AssistantState = {
  conversationId: null,
  messages: [],
  activeRun: null,
  lastPhases: [],
  sendError: null,
};

function patchMessage(state: AssistantState, runId: string, fn: (m: UiMessage) => UiMessage): AssistantState {
  let found = false;
  const messages = state.messages.map((m) => {
    if (m.role === "assistant" && m.runId === runId) {
      found = true;
      return fn(m);
    }
    return m;
  });
  if (!found) {
    // Un evento puede llegar antes de que `send_message` devuelva: se crea la burbuja aquí.
    messages.push(fn({ id: `a-${runId}`, role: "assistant", text: "", runId, streaming: true, steps: [] }));
  }
  return { ...state, messages };
}

function patchStep(m: UiMessage, callId: string, fn: (s: StepView) => StepView): UiMessage {
  return { ...m, steps: m.steps.map((s) => (s.callId === callId ? fn(s) : s)) };
}

/** Reductor puro de eventos del agente. */
export function applyAgentEvent(state: AssistantState, e: AgentEvent): AssistantState {
  switch (e.type) {
    case "run_started":
      return {
        ...patchMessage(state, e.runId, (m) => m),
        conversationId: e.conversationId,
        activeRun: { runId: e.runId, phase: null, phasesSeen: [] },
      };
    case "phase": {
      const run = state.activeRun?.runId === e.runId ? state.activeRun : { runId: e.runId, phase: null, phasesSeen: [] };
      const seen = run.phasesSeen.includes(e.phase) ? run.phasesSeen : [...run.phasesSeen, e.phase];
      return { ...state, activeRun: { ...run, phase: e.phase, phasesSeen: seen } };
    }
    case "token":
      return patchMessage(state, e.runId, (m) => ({ ...m, text: m.text + e.text }));
    case "tool_call":
      return patchMessage(state, e.runId, (m) => ({
        ...m,
        // El texto previo a pedir herramientas era el "preámbulo"; la respuesta final lo sustituye.
        steps: [...m.steps, { callId: e.callId, tool: e.tool, arguments: e.arguments, risk: e.risk, status: "pending" }],
      }));
    case "approval_required":
      return patchMessage(state, e.runId, (m) =>
        patchStep(m, e.callId, (s) => ({ ...s, status: "awaiting", approvalId: e.approvalId, reason: e.reason })),
      );
    case "approval_resolved":
      return patchMessage(state, e.runId, (m) => ({
        ...m,
        steps: m.steps.map((s) =>
          s.approvalId === e.approvalId
            ? { ...s, status: e.approved ? "running" : "denied", reason: e.approved ? s.reason : (e.reason ?? s.reason) }
            : s,
        ),
      }));
    case "tool_result":
      return patchMessage(state, e.runId, (m) =>
        patchStep(m, e.callId, (s) => ({
          ...s,
          status: s.status === "denied" ? "denied" : e.ok ? "done" : "failed",
          output: e.output,
          error: e.error,
          verification: e.verification,
          durationMs: e.durationMs,
        })),
      );
    case "final": {
      const next = patchMessage(state, e.runId, (m) => ({ ...m, text: e.text, streaming: false }));
      return finishRun(next, e.runId);
    }
    case "error": {
      const next = patchMessage(state, e.runId, (m) => ({
        ...m,
        streaming: false,
        error: { code: e.code, message: e.message },
      }));
      return finishRun(next, e.runId);
    }
    case "cancelled": {
      const next = patchMessage(state, e.runId, (m) => ({
        ...m,
        streaming: false,
        cancelled: true,
        steps: m.steps.map((s) => (s.status === "awaiting" || s.status === "pending" || s.status === "running" ? { ...s, status: "denied", reason: "cancelada" } : s)),
      }));
      return finishRun(next, e.runId);
    }
  }
}

function finishRun(state: AssistantState, runId: string): AssistantState {
  if (state.activeRun?.runId !== runId) return state;
  return { ...state, lastPhases: state.activeRun.phasesSeen, activeRun: null };
}

export function storedToUi(m: StoredMessage): UiMessage | null {
  if (m.role !== "user" && m.role !== "assistant") return null;
  return { id: m.id, role: m.role, text: m.content, steps: [] };
}

interface AssistantActions {
  send(text: string, opts?: { spoken?: boolean }): Promise<void>;
  cancel(): Promise<void>;
  resolveApproval(approvalId: string, approved: boolean): Promise<void>;
  openConversation(id: string): Promise<void>;
  newConversation(): void;
  markSpoken(messageId: string): void;
}

export const useAssistant = create<AssistantState & AssistantActions>((set, get) => ({
  ...initialAssistantState,

  async send(text, opts) {
    const trimmed = text.trim();
    if (!trimmed || get().activeRun) return;
    const userMsg: UiMessage = { id: `u-${Date.now()}`, role: "user", text: trimmed, steps: [] };
    set((s) => ({ messages: [...s.messages, userMsg], sendError: null }));
    try {
      const info = await backend().sendMessage(get().conversationId, trimmed, (e) => set((s) => applyAgentEvent(s, e)));
      set((s) => {
        const next = patchMessage(s, info.runId, (m) => ({ ...m, spoken: opts?.spoken ?? false }));
        return { ...next, conversationId: info.conversationId };
      });
    } catch (e) {
      set({ sendError: e instanceof Error ? e.message : String(e) });
    }
  },

  async cancel() {
    const run = get().activeRun;
    if (run) await backend().cancelRun(run.runId);
  },

  async resolveApproval(approvalId, approved) {
    await backend().resolveApproval(approvalId, approved);
  },

  async openConversation(id) {
    const stored = await backend().getMessages(id);
    set({
      conversationId: id,
      messages: stored.map(storedToUi).filter((m): m is UiMessage => m !== null),
      activeRun: null,
      sendError: null,
    });
  },

  newConversation() {
    if (get().activeRun) return;
    set({ conversationId: null, messages: [], sendError: null });
  },

  markSpoken(messageId) {
    set((s) => ({ messages: s.messages.map((m) => (m.id === messageId ? { ...m, spokenHandled: true } : m)) }));
  },
}));
