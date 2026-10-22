import { describe, expect, it } from "vitest";
import type { GraphEdge, GraphNode } from "@jarvis/shared-types";
import { fitAroundCore, kindCenter, layoutGraph } from "./layout";

const nodes: GraphNode[] = Array.from({ length: 60 }, (_, i) => ({
  id: `n${i}`, kind: i % 3 === 0 ? "entity" : "semantic", label: "", confidence: 1, verification: "verified", degree: 1, createdAt: "",
}));
const edges: GraphEdge[] = nodes.slice(1).map((n, i) => ({
  id: `e${i}`, source: n.id, target: "n0", kind: "BELONGS_TO", confidence: 1, provenance: "user", verified: true,
}));

describe("layout", () => {
  it("es determinista", () => {
    expect([...layoutGraph(nodes, edges)]).toEqual([...layoutGraph(nodes, edges)]);
  });

  it("deja libre el centro para el núcleo y no produce NaN", () => {
    const pos = layoutGraph(nodes, edges);
    for (const [, p] of pos) {
      expect(p.every(Number.isFinite)).toBe(true);
    }
    const avgDist = [...pos.values()].reduce((a, p) => a + Math.hypot(...p), 0) / pos.size;
    expect(avgDist).toBeGreaterThan(1.2);
    expect(Math.hypot(...kindCenter("entity"))).toBeGreaterThan(2);
  });

  it("conserva las posiciones previas como punto de partida", () => {
    const first = layoutGraph(nodes, edges);
    const again = layoutGraph(nodes, edges, first, 0);
    expect(again.get("n5")).toEqual(first.get("n5"));
  });

  it("aguanta cientos de nodos en un tiempo razonable", () => {
    const many: GraphNode[] = Array.from({ length: 400 }, (_, i) => ({ ...nodes[0], id: `m${i}` }));
    const t = performance.now();
    layoutGraph(many, []);
    expect(performance.now() - t).toBeLessThan(1500);
  });

  it("centra la nube en el núcleo y deja su espacio libre", () => {
    const fitted = fitAroundCore(layoutGraph(nodes, edges), 4, 0.9);
    const pts = [...fitted.values()];
    const c = pts.reduce((a, p) => [a[0] + p[0], a[1] + p[1], a[2] + p[2]], [0, 0, 0]).map((v) => v / pts.length);
    expect(Math.hypot(...c)).toBeLessThan(0.6);
    expect(Math.min(...pts.map((p) => Math.hypot(...p)))).toBeGreaterThanOrEqual(0.9 - 1e-6);
  });
});
