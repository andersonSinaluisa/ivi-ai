// Núcleo de Jarvis: siete arcos, uno por fase del ciclo, que se encienden mientras trabaja.
// El disco central respira con el nivel del micrófono.
import { useEffect, useRef } from "react";
import type { AgentPhase, VoiceState } from "@jarvis/shared-types";

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

interface Props {
  size: number;
  phase: AgentPhase | null;
  phasesSeen: AgentPhase[];
  busy: boolean;
  level: number;
  voiceState: VoiceState;
  awaitingApproval?: boolean;
}

const CYAN = "34, 211, 238";
const AMBER = "245, 181, 68";
const TIDE = "18, 48, 90";

export function CoreRing({ size, phase, phasesSeen, busy, level, voiceState, awaitingApproval }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);
  const props = useRef({ phase, phasesSeen, busy, level, voiceState, awaitingApproval });
  props.current = { phase, phasesSeen, busy, level, voiceState, awaitingApproval };

  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext?.("2d");
    if (!canvas || !ctx) return;
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = size * dpr;
    canvas.height = size * dpr;
    ctx.scale(dpr, dpr);

    let raf = 0;
    let smoothLevel = 0;
    const c = size / 2;
    const radius = size * 0.4;
    const gap = 0.07;
    const seg = (Math.PI * 2) / PHASES.length;

    const draw = (t: number) => {
      const p = props.current;
      smoothLevel += (p.level - smoothLevel) * 0.25;
      const pulse = reduced ? 0.5 : 0.5 + 0.5 * Math.sin(t / 380);
      ctx.clearRect(0, 0, size, size);

      // Arcos de fase
      PHASES.forEach((ph, i) => {
        const start = -Math.PI / 2 + i * seg + gap / 2;
        const end = start + seg - gap;
        const isCurrent = p.busy && p.phase === ph;
        const seen = p.phasesSeen.includes(ph);
        const color = isCurrent && ph === "authorize" && p.awaitingApproval ? AMBER : CYAN;
        let alpha = 0.18;
        if (seen) alpha = 0.55;
        if (isCurrent) alpha = 0.7 + 0.3 * pulse;
        ctx.beginPath();
        ctx.arc(c, c, radius, start, end);
        ctx.lineWidth = isCurrent ? size * 0.035 : size * 0.022;
        ctx.lineCap = "round";
        ctx.strokeStyle = seen || isCurrent ? `rgba(${color}, ${alpha})` : `rgba(${TIDE}, 0.9)`;
        ctx.shadowColor = isCurrent ? `rgba(${color}, 0.8)` : "transparent";
        ctx.shadowBlur = isCurrent ? 18 : 0;
        ctx.stroke();
      });
      ctx.shadowBlur = 0;

      // Anillo interior fino
      ctx.beginPath();
      ctx.arc(c, c, radius * 0.72, 0, Math.PI * 2);
      ctx.lineWidth = 1;
      ctx.strokeStyle = `rgba(${CYAN}, 0.18)`;
      ctx.stroke();

      // Disco central: crece con la voz, late suave al pensar
      const listening = p.voiceState === "RECORDING_COMMAND";
      const speaking = p.voiceState === "SPEAKING";
      const base = radius * 0.34;
      const grow = listening ? smoothLevel * radius * 0.3 : speaking || p.busy ? pulse * radius * 0.05 : 0;
      const r = base + grow;
      const grad = ctx.createRadialGradient(c, c, r * 0.1, c, c, r);
      grad.addColorStop(0, `rgba(${CYAN}, ${p.busy || listening || speaking ? 0.95 : 0.6})`);
      grad.addColorStop(1, `rgba(${CYAN}, 0.05)`);
      ctx.beginPath();
      ctx.arc(c, c, r, 0, Math.PI * 2);
      ctx.fillStyle = grad;
      ctx.fill();

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [size]);

  const label = busy && phase ? `Jarvis está en la fase ${PHASE_LABEL[phase]}` : "Jarvis en espera";
  return <canvas ref={ref} role="img" aria-label={label} style={{ width: size, height: size }} />;
}
