// Estado del grafo de conocimiento y respuesta visual a los eventos.
import { create } from "zustand";
import type { BackendVisualEvent, GraphEdge, GraphNode } from "@jarvis/shared-types";
import { backend } from "../services/backend";
import { fitAroundCore, layoutGraph, placeNode, type Vec3 } from "../features/neural/layout";

/** Duración de un resaltado (recuperación, conexión). */
export const HIGHLIGHT_MS = 4000;
/** Duración de la animación de nacimiento de un nodo. */
export const BIRTH_MS = 1200;
/** Destello del núcleo al completar una tarea o aprender una habilidad. */
export const FLASH_MS = 1100;

export type RenderMode = "3d" | "2d";

export interface GraphState {
  nodes: GraphNode[];
  edges: GraphEdge[];
  positions: Map<string, Vec3>;
  /** id de nodo o relación → instante del resaltado (ms). */
  highlights: Record<string, number>;
  births: Record<string, number>;
  /** Nodos que llegaron del Cloud Core en una búsqueda y aún no están en este equipo. */
  remote: Record<string, true>;
  selectedId: string | null;
  taskActive: boolean;
  flashAt: number | null;
  /** Cambió la topología: hay que reconstruir los buffers. */
  version: number;
  needsReload: boolean;
  total: number;
  truncated: boolean;
}

export const initialGraphState: GraphState = {
  nodes: [],
  edges: [],
  positions: new Map(),
  highlights: {},
  births: {},
  remote: {},
  selectedId: null,
  taskActive: false,
  flashAt: null,
  version: 0,
  needsReload: false,
  total: 0,
  truncated: false,
};

/** Intensidad 0..1 de un resaltado que empezó en `start`. */
export function intensity(start: number | undefined, now: number, duration = HIGHLIGHT_MS): number {
  if (start === undefined) return 0;
  const t = (now - start) / duration;
  return t < 0 || t >= 1 ? 0 : 1 - t * t;
}

/** Progreso 0..1 de la animación de nacimiento (1 = nodo normal). */
export function birthProgress(start: number | undefined, now: number): number {
  if (start === undefined) return 1;
  return Math.min(1, Math.max(0, (now - start) / BIRTH_MS));
}

function upsertEdges(edges: GraphEdge[], incoming: GraphEdge[], nodeIds: Set<string>): GraphEdge[] {
  const byId = new Map(edges.map((e) => [e.id, e]));
  for (const e of incoming) {
    if (nodeIds.has(e.source) && nodeIds.has(e.target)) byId.set(e.id, e);
  }
  return [...byId.values()];
}

/** Reductor puro: cada evento produce un cambio visual coherente. */
export function applyVisualEvent(s: GraphState, e: BackendVisualEvent, now: number): GraphState {
  switch (e.type) {
    case "MEMORY_CREATED": {
      const exists = s.nodes.some((n) => n.id === e.node.id);
      const nodes = exists ? s.nodes.map((n) => (n.id === e.node.id ? e.node : n)) : [...s.nodes, e.node];
      const ids = new Set(nodes.map((n) => n.id));
      const edges = upsertEdges(s.edges, e.edges, ids);
      const positions = new Map(s.positions);
      if (!positions.has(e.node.id)) positions.set(e.node.id, placeNode(e.node, edges, positions));
      const highlights = { ...s.highlights, [e.node.id]: now };
      for (const ed of e.edges) highlights[ed.id] = now;
      return {
        ...s,
        nodes,
        edges,
        positions,
        highlights,
        births: { ...s.births, [e.node.id]: now },
        version: s.version + 1,
        total: exists ? s.total : s.total + 1,
      };
    }
    case "MEMORY_RETRIEVED": {
      const highlights = { ...s.highlights };
      for (const id of e.memoryIds) highlights[id] = now;
      for (const id of e.relationIds) highlights[id] = now;
      // Las conexiones entre recuerdos recuperados también se iluminan.
      const set = new Set(e.memoryIds);
      for (const ed of s.edges) if (set.has(ed.source) && set.has(ed.target)) highlights[ed.id] = now;
      // Si alguno no está en la vista, conviene recargarla.
      const missing = e.memoryIds.some((id) => !s.nodes.some((n) => n.id === id));
      return { ...s, highlights, needsReload: s.needsReload || missing };
    }
    case "MEMORY_RETRIEVED_REMOTE": {
      // Recuerdos que solo estaban en el Cloud Core: aparecen en el grafo, marcados como remotos y resaltados.
      const known = new Set(s.nodes.map((n) => n.id));
      const fresh = e.nodes.filter((n) => !known.has(n.id));
      const nodes = [...s.nodes, ...fresh];
      const ids = new Set(nodes.map((n) => n.id));
      const edges = upsertEdges(s.edges, e.edges, ids);
      const positions = new Map(s.positions);
      const highlights = { ...s.highlights };
      const births = { ...s.births };
      const remote = { ...s.remote };
      for (const n of fresh) {
        positions.set(n.id, placeNode(n, edges, positions));
        births[n.id] = now;
        remote[n.id] = true;
      }
      for (const n of e.nodes) highlights[n.id] = now;
      for (const id of e.relationIds) highlights[id] = now;
      const changed = fresh.length > 0 || edges.length !== s.edges.length;
      return { ...s, nodes, edges, positions, highlights, births, remote, version: changed ? s.version + 1 : s.version };
    }
    case "MEMORY_UPDATED":
      return {
        ...s,
        nodes: s.nodes.map((n) => (n.id === e.node.id ? e.node : n)),
        highlights: { ...s.highlights, [e.node.id]: now },
      };
    case "MEMORY_DELETED": {
      const positions = new Map(s.positions);
      positions.delete(e.memoryId);
      return {
        ...s,
        nodes: s.nodes.filter((n) => n.id !== e.memoryId),
        edges: s.edges.filter((ed) => ed.source !== e.memoryId && ed.target !== e.memoryId),
        positions,
        selectedId: s.selectedId === e.memoryId ? null : s.selectedId,
        version: s.version + 1,
        total: Math.max(0, s.total - 1),
      };
    }
    case "MEMORY_CONNECTED": {
      const ids = new Set(s.nodes.map((n) => n.id));
      const known = s.edges.some((x) => x.id === e.edge.id);
      return {
        ...s,
        edges: upsertEdges(s.edges, [e.edge], ids),
        highlights: { ...s.highlights, [e.edge.id]: now, [e.edge.source]: now, [e.edge.target]: now },
        version: known ? s.version : s.version + 1,
      };
    }
    case "MEMORY_CONSOLIDATED":
    case "MEMORY_IMPORTED":
      return { ...s, needsReload: true };
    case "SKILL_LEARNED":
      return { ...s, flashAt: now };
    case "TASK_STARTED":
      return { ...s, taskActive: true };
    case "TASK_COMPLETED":
      return { ...s, taskActive: false, flashAt: now };
  }
}

interface GraphActions {
  mode: RenderMode;
  lowPower: boolean;
  loading: boolean;
  error: string | null;
  load(limit?: number): Promise<void>;
  apply(e: BackendVisualEvent): void;
  select(id: string | null): void;
  setMode(mode: RenderMode): void;
  setLowPower(on: boolean): void;
}

function readPref<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}

function writePref(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* sin almacenamiento: no pasa nada */
  }
}

export const useGraph = create<GraphState & GraphActions>((set, get) => ({
  ...initialGraphState,
  mode: readPref<RenderMode>("jarvis.graph.mode", "3d"),
  lowPower: readPref("jarvis.graph.lowPower", false),
  loading: false,
  error: null,

  async load(limit) {
    set({ loading: true, error: null });
    try {
      const view = await backend().memoryGraph({ limit: limit ?? (get().lowPower ? 150 : 400) });
      // Si ya había posiciones se parte de ellas (sin reescalar) para que el grafo no salte.
      const previous = get().positions;
      const laid = layoutGraph(view.nodes, view.edges, previous);
      const positions = previous.size === 0 ? fitAroundCore(laid) : laid;
      set((s) => ({
        nodes: view.nodes,
        edges: view.edges,
        positions,
        total: view.totalMemories,
        truncated: view.truncated,
        // Tras recargar, lo que vino del Cloud ya está sincronizado en local (o no está en la vista).
        remote: {},
        version: s.version + 1,
        needsReload: false,
        loading: false,
      }));
    } catch (e) {
      set({ loading: false, error: e instanceof Error ? e.message : String(e) });
    }
  },

  apply(e) {
    set((s) => applyVisualEvent(s, e, performance.now()));
  },

  select(id) {
    set({ selectedId: id });
  },

  setMode(mode) {
    writePref("jarvis.graph.mode", mode);
    set({ mode });
  },

  setLowPower(on) {
    writePref("jarvis.graph.lowPower", on);
    set({ lowPower: on });
  },
}));
