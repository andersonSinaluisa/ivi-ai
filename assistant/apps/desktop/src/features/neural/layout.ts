// Disposición 3D del grafo de conocimiento. Determinista (mismo grafo → mismas posiciones),
// agrupada por tipo de recuerdo y relajada con un modelo de fuerzas sencillo.
import type { GraphEdge, GraphNode, MemoryKind } from "@jarvis/shared-types";

export type Vec3 = [number, number, number];

export const KIND_COLOR: Record<MemoryKind, string> = {
  semantic: "#22d3ee",
  entity: "#a78bfa",
  procedural: "#34d399",
  experience: "#f5b544",
  episodic: "#60a5fa",
};

export const KIND_LABEL: Record<MemoryKind, string> = {
  semantic: "Hechos",
  entity: "Entidades",
  procedural: "Procedimientos",
  experience: "Experiencias",
  episodic: "Episodios",
};

const KINDS: MemoryKind[] = ["entity", "semantic", "procedural", "experience", "episodic"];

/** Hash FNV-1a de 32 bits. */
export function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function seeded(id: string): () => number {
  let a = hash(id) || 1;
  return () => {
    a ^= a << 13;
    a ^= a >>> 17;
    a ^= a << 5;
    return ((a >>> 0) % 100000) / 100000;
  };
}

/** Centro de cada tipo: puntos repartidos sobre una esfera (Fibonacci). */
export function kindCenter(kind: MemoryKind, radius = 3.4): Vec3 {
  const i = KINDS.indexOf(kind);
  const n = KINDS.length;
  const y = 1 - (2 * (i + 0.5)) / n;
  const r = Math.sqrt(1 - y * y);
  const theta = Math.PI * (3 - Math.sqrt(5)) * i;
  return [Math.cos(theta) * r * radius, y * radius * 0.75, Math.sin(theta) * r * radius];
}

function jitter(id: string, spread: number): Vec3 {
  const r = seeded(id);
  // Punto aleatorio dentro de una esfera.
  const u = r() * 2 - 1;
  const t = r() * Math.PI * 2;
  const d = Math.cbrt(r()) * spread;
  const s = Math.sqrt(1 - u * u);
  return [Math.cos(t) * s * d, u * d, Math.sin(t) * s * d];
}

const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];

/** Posición de un nodo nuevo: junto a sus vecinos ya colocados, o en el grupo de su tipo. */
export function placeNode(node: GraphNode, edges: GraphEdge[], positions: Map<string, Vec3>): Vec3 {
  const neighbors = edges
    .filter((e) => e.source === node.id || e.target === node.id)
    .map((e) => positions.get(e.source === node.id ? e.target : e.source))
    .filter((p): p is Vec3 => Boolean(p));
  if (neighbors.length) {
    const c = neighbors.reduce((acc, p) => add(acc, p), [0, 0, 0] as Vec3).map((v) => v / neighbors.length) as Vec3;
    return add(c, jitter(node.id, 0.45));
  }
  return add(kindCenter(node.kind), jitter(node.id, 1.5));
}

/**
 * Coloca todos los nodos. Los que ya tenían posición (`previous`) parten de ella para que el
 * grafo no "salte" al recargar.
 */
export function layoutGraph(nodes: GraphNode[], edges: GraphEdge[], previous?: Map<string, Vec3>, iterations?: number): Map<string, Vec3> {
  const n = nodes.length;
  const index = new Map(nodes.map((nd, i) => [nd.id, i]));
  const pos: Vec3[] = nodes.map((nd) => previous?.get(nd.id) ?? add(kindCenter(nd.kind), jitter(nd.id, 1.5)));
  const centers = nodes.map((nd) => kindCenter(nd.kind));
  const links = edges
    .map((e) => [index.get(e.source), index.get(e.target), e.confidence] as const)
    .filter((l): l is readonly [number, number, number] => l[0] !== undefined && l[1] !== undefined);
  const iters = iterations ?? (n > 400 ? 20 : n > 150 ? 40 : 60);
  const repulsion = 0.012;
  for (let it = 0; it < iters; it++) {
    const cooling = 1 - it / (iters + 1);
    const force: Vec3[] = pos.map(() => [0, 0, 0]);
    // Repulsión (O(n²); n está limitado por la vista paginada).
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const dx = pos[i][0] - pos[j][0];
        const dy = pos[i][1] - pos[j][1];
        const dz = pos[i][2] - pos[j][2];
        const d2 = dx * dx + dy * dy + dz * dz + 0.01;
        if (d2 > 4) continue;
        const f = repulsion / d2;
        force[i][0] += dx * f;
        force[i][1] += dy * f;
        force[i][2] += dz * f;
        force[j][0] -= dx * f;
        force[j][1] -= dy * f;
        force[j][2] -= dz * f;
      }
    }
    // Muelles en las relaciones.
    for (const [a, b, conf] of links) {
      const dx = pos[b][0] - pos[a][0];
      const dy = pos[b][1] - pos[a][1];
      const dz = pos[b][2] - pos[a][2];
      const d = Math.sqrt(dx * dx + dy * dy + dz * dz) + 1e-6;
      const f = 0.04 * (0.2 + conf) * (d - 0.8);
      force[a][0] += (dx / d) * f;
      force[a][1] += (dy / d) * f;
      force[a][2] += (dz / d) * f;
      force[b][0] -= (dx / d) * f;
      force[b][1] -= (dy / d) * f;
      force[b][2] -= (dz / d) * f;
    }
    // Atracción suave al centro de su tipo.
    for (let i = 0; i < n; i++) {
      for (let k = 0; k < 3; k++) {
        force[i][k] += (centers[i][k] - pos[i][k]) * 0.015;
        pos[i][k] += Math.max(-0.3, Math.min(0.3, force[i][k])) * cooling;
      }
    }
  }
  return new Map(nodes.map((nd, i) => [nd.id, pos[i]]));
}

/**
 * Centra la nube alrededor del núcleo (origen), la escala para ocupar la vista y deja libre
 * una pequeña esfera central para el núcleo.
 */
export function fitAroundCore(positions: Map<string, Vec3>, radius = 4.2, clear = 0.9): Map<string, Vec3> {
  const pts = [...positions.values()];
  if (pts.length === 0) return positions;
  const c = pts.reduce((a, p) => add(a, p), [0, 0, 0] as Vec3).map((v) => v / pts.length) as Vec3;
  const centered = [...positions].map(([id, p]) => [id, [p[0] - c[0], p[1] - c[1], p[2] - c[2]] as Vec3] as const);
  const dists = centered.map(([, p]) => Math.hypot(...p)).sort((a, b) => a - b);
  const p95 = dists[Math.min(dists.length - 1, Math.floor(dists.length * 0.95))] || 1;
  const k = pts.length === 1 ? 1 : radius / p95;
  return new Map(
    centered.map(([id, p]) => {
      const q: Vec3 = [p[0] * k, p[1] * k, p[2] * k];
      const d = Math.hypot(...q);
      if (d < clear) {
        const dir: Vec3 = d < 1e-6 ? jitter(id, 1) : q;
        const n = Math.hypot(...dir) || 1;
        return [id, [(dir[0] / n) * clear, (dir[1] / n) * clear, (dir[2] / n) * clear] as Vec3];
      }
      return [id, q];
    }),
  );
}
