import type { AgentPhase } from "@jarvis/shared-types";

export const PHASES: AgentPhase[] = ["observe", "understand", "plan", "authorize", "execute", "verify", "remember"];

export const PHASE_LABEL: Record<AgentPhase, string> = {
  observe: "Observar",
  understand: "Entender",
  plan: "Planificar",
  authorize: "Autorizar",
  execute: "Ejecutar",
  verify: "Verificar",
  remember: "Recordar",
};
