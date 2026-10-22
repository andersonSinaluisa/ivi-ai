import { describe, expect, it } from "vitest";
import type { GraphEdge, GraphNode } from "@jarvis/shared-types";
import { applyVisualEvent, birthProgress, BIRTH_MS, HIGHLIGHT_MS, initialGraphState, intensity, type GraphState } from "./graphStore";

const node = (id: string, kind: GraphNode["kind"] = "semantic"): GraphNode => ({
  id, kind, label: id, confidence: 1, verification: "verified", degree: 0, createdAt: "2026-10-22T00:00:00Z",
});
const edge = (id: string, source: string, target: string): GraphEdge => ({
  id, source, target, kind: "RELATED_TO", confidence: 0.8, provenance: "inferred", verified: false,
});

function base(): GraphState {
  let s = applyVisualEvent(initialGraphState, { type: "MEMORY_CREATED", node: node("a", "entity"), edges: [] }, 0);
  s = applyVisualEvent(s, { type: "MEMORY_CREATED", node: node("b"), edges: [edge("e1", "b", "a")] }, 10);
  return s;
}

describe("eventos visuales", () => {
  it("MEMORY_CREATED añade el nodo cerca de sus vecinos, con animación de nacimiento", () => {
    const s = base();
    expect(s.nodes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(s.edges).toHaveLength(1);
    expect(s.births.b).toBe(10);
    expect(s.highlights.e1).toBe(10);
    const [ax, ay, az] = s.positions.get("a")!;
    const [bx, by, bz] = s.positions.get("b")!;
    expect(Math.hypot(ax - bx, ay - by, az - bz)).toBeLessThan(1.2);
    expect(s.version).toBe(2);
    expect(s.total).toBe(2);
  });

  it("MEMORY_RETRIEVED ilumina los recuerdos y las conexiones entre ellos", () => {
    const s = applyVisualEvent(base(), { type: "MEMORY_RETRIEVED", runId: "r", memoryIds: ["a", "b"], relationIds: [] }, 500);
    expect(s.highlights.a).toBe(500);
    expect(s.highlights.e1).toBe(500);
    expect(s.needsReload).toBe(false);
    const missing = applyVisualEvent(s, { type: "MEMORY_RETRIEVED", runId: "r", memoryIds: ["zzz"], relationIds: [] }, 600);
    expect(missing.needsReload).toBe(true);
  });

  it("MEMORY_DELETED quita el nodo, sus relaciones y la selección", () => {
    const sel = { ...base(), selectedId: "a" };
    const s = applyVisualEvent(sel, { type: "MEMORY_DELETED", memoryId: "a" }, 0);
    expect(s.nodes.map((n) => n.id)).toEqual(["b"]);
    expect(s.edges).toHaveLength(0);
    expect(s.selectedId).toBeNull();
    expect(s.positions.has("a")).toBe(false);
  });

  it("MEMORY_CONNECTED añade la relación solo si ambos extremos están en la vista", () => {
    const s = applyVisualEvent(base(), { type: "MEMORY_CONNECTED", edge: { ...edge("e2", "a", "b"), kind: "DEPENDS_ON" } }, 50);
    expect(s.edges.map((e) => e.id)).toContain("e2");
    const outside = applyVisualEvent(base(), { type: "MEMORY_CONNECTED", edge: edge("e3", "a", "fuera") }, 50);
    expect(outside.edges.map((e) => e.id)).not.toContain("e3");
  });

  it("tareas, habilidades e importaciones", () => {
    let s = applyVisualEvent(base(), { type: "TASK_STARTED", runId: "r", goal: "x" }, 0);
    expect(s.taskActive).toBe(true);
    s = applyVisualEvent(s, { type: "TASK_COMPLETED", runId: "r", status: "completed" }, 100);
    expect(s.taskActive).toBe(false);
    expect(s.flashAt).toBe(100);
    s = applyVisualEvent(s, { type: "SKILL_LEARNED", skillId: "k", name: "n" }, 200);
    expect(s.flashAt).toBe(200);
    s = applyVisualEvent(s, { type: "MEMORY_IMPORTED", imported: 3, restored: false }, 300);
    expect(s.needsReload).toBe(true);
  });

  it("los resaltados se apagan y el nacimiento termina (sin animaciones eternas)", () => {
    expect(intensity(0, 0)).toBe(1);
    expect(intensity(0, HIGHLIGHT_MS / 2)).toBeGreaterThan(0);
    expect(intensity(0, HIGHLIGHT_MS)).toBe(0);
    expect(intensity(undefined, 10)).toBe(0);
    expect(birthProgress(0, BIRTH_MS * 2)).toBe(1);
    expect(birthProgress(undefined, 0)).toBe(1);
  });
});

describe("recuerdos traídos del Cloud Core", () => {
  it("MEMORY_RETRIEVED_REMOTE añade los nodos remotos marcados, resaltados y conectados", () => {
    const s = applyVisualEvent(
      base(),
      { type: "MEMORY_RETRIEVED_REMOTE", runId: "r", nodes: [node("cloud-1")], edges: [edge("e-r", "cloud-1", "a")], relationIds: ["e-r"] },
      700,
    );
    expect(s.nodes.map((n) => n.id)).toContain("cloud-1");
    expect(s.remote["cloud-1"]).toBe(true);
    expect(s.remote.a).toBeUndefined();
    expect(s.highlights["cloud-1"]).toBe(700);
    expect(s.highlights["e-r"]).toBe(700);
    expect(s.births["cloud-1"]).toBe(700);
    expect(s.edges.some((e) => e.id === "e-r")).toBe(true);
    expect(s.positions.has("cloud-1")).toBe(true);
    expect(s.version).toBe(3);
  });

  it("si el nodo ya está en local solo se ilumina (no se duplica ni se marca como remoto)", () => {
    const s = applyVisualEvent(base(), { type: "MEMORY_RETRIEVED_REMOTE", runId: null, nodes: [node("a")], edges: [], relationIds: [] }, 800);
    expect(s.nodes.filter((n) => n.id === "a")).toHaveLength(1);
    expect(s.remote.a).toBeUndefined();
    expect(s.highlights.a).toBe(800);
    expect(s.version).toBe(2);
  });

  it("una relación hacia un nodo que no está en la vista se descarta", () => {
    const s = applyVisualEvent(
      base(),
      { type: "MEMORY_RETRIEVED_REMOTE", runId: null, nodes: [node("c2")], edges: [edge("e-x", "c2", "nadie")], relationIds: ["e-x"] },
      900,
    );
    expect(s.edges.some((e) => e.id === "e-x")).toBe(false);
  });
});
