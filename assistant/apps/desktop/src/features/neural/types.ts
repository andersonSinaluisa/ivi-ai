// Contratos de las vistas del grafo (sin dependencias de Three.js, para poder cargarlo aparte).
import type { GraphEdge, GraphNode } from "@jarvis/shared-types";
import type { Vec3 } from "./layout";

export interface CoreState {
  /** 0..1: amplitud de la voz de Jarvis o nivel del micrófono. */
  level: number;
  listening: boolean;
  speaking: boolean;
  busy: boolean;
  awaitingApproval: boolean;
  flashAt: number | null;
}

export interface NeuralViewProps {
  nodes: GraphNode[];
  edges: GraphEdge[];
  positions: Map<string, Vec3>;
  highlights: Record<string, number>;
  births: Record<string, number>;
  /** Nodos traídos del Cloud Core (se dibujan con un anillo/tono distinto). */
  remote?: Record<string, true>;
  selectedId: string | null;
  version: number;
  core: CoreState;
  lowPower: boolean;
  onSelect(id: string | null): void;
}

/** ¿Hay WebGL? Si no, se usa la vista 2D. */
export function supportsWebGL(): boolean {
  try {
    const c = document.createElement("canvas");
    return Boolean(c.getContext("webgl2") ?? c.getContext("webgl"));
  } catch {
    return false;
  }
}
