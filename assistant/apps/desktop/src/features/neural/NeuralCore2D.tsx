// Alternativa 2D (Canvas 2D): para equipos sin WebGL, para el modo de bajo consumo extremo y
// para vistas pequeñas como la de relaciones en la pantalla de Memoria. Un solo <canvas>.
import { useEffect, useRef } from "react";
import { KIND_COLOR } from "./layout";
import type { NeuralViewProps } from "./types";
import { birthProgress, FLASH_MS, intensity } from "../../stores/graphStore";

type Props = Omit<NeuralViewProps, "version"> & { height?: number; label?: string };

export function NeuralCore2D(props: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const live = useRef(props);
  live.current = props;
  const projected = useRef<{ id: string; x: number; y: number }[]>([]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext?.("2d");
    if (!canvas || !ctx) return;
    let raf = 0;
    let lastActive = true;

    const draw = () => {
      const p = live.current;
      const now = performance.now();
      const rect = canvas.getBoundingClientRect();
      const dpr = p.lowPower ? 1 : Math.min(2, window.devicePixelRatio || 1);
      const w = Math.max(1, rect.width);
      const h = Math.max(1, rect.height);
      if (canvas.width !== Math.round(w * dpr)) {
        canvas.width = Math.round(w * dpr);
        canvas.height = Math.round(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      // Proyección ortográfica con una rotación lenta (si no es bajo consumo).
      const angle = p.lowPower ? 0.4 : now / 20000;
      const cos = Math.cos(angle);
      const sin = Math.sin(angle);
      const scale = Math.min(w, h) / 10;
      const cx = w / 2;
      const cy = h / 2;
      const pts = new Map<string, { x: number; y: number; z: number }>();
      for (const n of p.nodes) {
        const v = p.positions.get(n.id);
        if (!v) continue;
        const x = v[0] * cos - v[2] * sin;
        const z = v[0] * sin + v[2] * cos;
        pts.set(n.id, { x: cx + x * scale, y: cy - v[1] * scale, z });
      }

      let active = false;
      ctx.lineWidth = 1;
      for (const e of p.edges) {
        const a = pts.get(e.source);
        const b = pts.get(e.target);
        if (!a || !b) continue;
        const hl = intensity(p.highlights[e.id], now);
        const sel = p.selectedId !== null && (e.source === p.selectedId || e.target === p.selectedId);
        const alpha = Math.max(0.06 + e.confidence * 0.14, hl * 0.9, sel ? 0.55 : 0) * (e.provenance === "inferred" && !e.verified ? 0.6 : 1);
        ctx.strokeStyle = hl > 0 ? `rgba(230,251,255,${alpha})` : `rgba(34,211,238,${alpha})`;
        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
        ctx.stroke();
        if (hl > 0.05) {
          active = true;
          const t = (now / 900) % 1;
          ctx.fillStyle = "rgba(230,251,255,0.9)";
          ctx.beginPath();
          ctx.arc(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, 1.8, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      const list: { id: string; x: number; y: number }[] = [];
      const sorted = [...p.nodes].sort((n1, n2) => (pts.get(n1.id)?.z ?? 0) - (pts.get(n2.id)?.z ?? 0));
      for (const n of sorted) {
        const pt = pts.get(n.id);
        if (!pt) continue;
        const hl = intensity(p.highlights[n.id], now);
        const born = birthProgress(p.births[n.id], now);
        if (hl > 0 || born < 1) active = true;
        const selected = p.selectedId === n.id;
        const r = (1.6 + Math.min(3, Math.sqrt(n.degree) * 0.7) + (n.kind === "entity" ? 1 : 0)) * born * (selected ? 1.6 : 1) * (1 + hl * 0.8);
        ctx.globalAlpha = n.verification === "verified" ? 1 : 0.7;
        if (hl > 0 || selected) {
          ctx.shadowColor = KIND_COLOR[n.kind];
          ctx.shadowBlur = 12 * Math.max(hl, selected ? 0.8 : 0);
        }
        ctx.fillStyle = hl > 0.4 ? "#e6fbff" : KIND_COLOR[n.kind];
        ctx.beginPath();
        ctx.arc(pt.x, pt.y, Math.max(0.5, r), 0, Math.PI * 2);
        ctx.fill();
        ctx.shadowBlur = 0;
        if (p.remote?.[n.id]) {
          // Anillo: este recuerdo vino del Cloud Core.
          ctx.strokeStyle = "#a78bfa";
          ctx.lineWidth = 1.2;
          ctx.beginPath();
          ctx.arc(pt.x, pt.y, Math.max(0.5, r) + 2.5, 0, Math.PI * 2);
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
        list.push({ id: n.id, x: pt.x, y: pt.y });
      }
      projected.current = list;

      // Núcleo
      const c = p.core;
      const core = 7 * (c.speaking ? 1 + c.level : c.listening ? 1.1 + 0.1 * Math.sin(now / 160) : 1);
      ctx.fillStyle = c.awaitingApproval ? "#f5b544" : "#22d3ee";
      ctx.globalAlpha = c.busy || c.listening || c.speaking ? 0.95 : 0.55;
      ctx.beginPath();
      ctx.arc(cx, cy, core, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
      if (c.flashAt !== null) {
        const f = (now - c.flashAt) / FLASH_MS;
        if (f >= 0 && f < 1) {
          active = true;
          ctx.strokeStyle = `rgba(230,251,255,${0.6 * (1 - f)})`;
          ctx.beginPath();
          ctx.arc(cx, cy, core + f * 60, 0, Math.PI * 2);
          ctx.stroke();
        }
      }

      lastActive = active || c.listening || c.speaking || c.busy;
      // En bajo consumo, sin animación pendiente, se redibuja a 2 fps.
      raf = p.lowPower && !lastActive ? window.setTimeout(() => (raf = requestAnimationFrame(draw)), 500) : requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(raf);
      clearTimeout(raf);
    };
  }, []);

  const onClick = (ev: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = ev.currentTarget.getBoundingClientRect();
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top;
    let best: string | null = null;
    let bestD = 12 * 12;
    for (const pt of projected.current) {
      const d = (pt.x - x) ** 2 + (pt.y - y) ** 2;
      if (d < bestD) {
        bestD = d;
        best = pt.id;
      }
    }
    props.onSelect(best);
  };

  return (
    <canvas
      ref={canvasRef}
      onClick={onClick}
      role="img"
      aria-label={props.label ?? `Grafo de conocimiento: ${props.nodes.length} recuerdos, ${props.edges.length} relaciones`}
      style={{ width: "100%", height: props.height ?? "100%", display: "block", cursor: "pointer" }}
    />
  );
}
