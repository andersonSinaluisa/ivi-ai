// Neural Core: grafo semántico de la memoria de Jarvis (no las neuronas de Qwen, que no son
// accesibles). Cada punto es un recuerdo real del Memory Engine; cada línea, una relación.
import { Suspense, lazy, useEffect, useMemo, useState } from "react";
import type { VoiceState } from "@jarvis/shared-types";
import { useGraph } from "../../stores/graphStore";
import { useVoice } from "../../stores/voiceStore";
import { useAssistant } from "../../stores/assistantStore";
import { useUi } from "../../stores/uiStore";
import { KIND_COLOR, KIND_LABEL } from "./layout";
import { NeuralCore2D } from "./NeuralCore2D";
import { supportsWebGL, type CoreState } from "./types";
import { formatDate } from "../../components/ui";

// Three.js se carga aparte: la vista 2D y el resto de la app no lo necesitan.
const NeuralCore3D = lazy(() => import("./NeuralCore3D").then((m) => ({ default: m.NeuralCore3D })));

const VERIFICATION_LABEL = { verified: "verificado", unverified: "sin verificar", disputed: "en duda" } as const;

/** Estado del núcleo a partir de la voz y del agente. */
export function deriveCore(
  voiceState: VoiceState,
  micLevel: number,
  outputLevel: number,
  taskActive: boolean,
  awaitingApproval: boolean,
  flashAt: number | null,
): CoreState {
  const listening = voiceState === "RECORDING_COMMAND" || voiceState === "WAKE_DETECTED";
  const speaking = voiceState === "SPEAKING";
  return {
    listening,
    speaking,
    level: speaking ? outputLevel : listening ? micLevel : 0,
    busy: taskActive || voiceState === "THINKING" || voiceState === "TRANSCRIBING",
    awaitingApproval,
    flashAt,
  };
}

export function NeuralCore({ height = 460 }: { height?: number }) {
  const g = useGraph();
  const voice = useVoice();
  const awaiting = useAssistant((s) => s.messages.some((m) => m.steps.some((st) => st.status === "awaiting")));
  const go = useUi((s) => s.go);
  const openMemory = useUi((s) => s.openMemory);
  const [webgl] = useState(() => supportsWebGL());
  const mode = webgl ? g.mode : "2d";

  useEffect(() => {
    void g.load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [g.lowPower]);

  // Recarga diferida cuando un evento lo pide (importación, consolidación, recuerdos fuera de vista).
  useEffect(() => {
    if (!g.needsReload) return;
    const t = setTimeout(() => void useGraph.getState().load(), 800);
    return () => clearTimeout(t);
  }, [g.needsReload]);

  const core = deriveCore(voice.state, voice.level, voice.outputLevel, g.taskActive, awaiting, g.flashAt);
  const selected = useMemo(() => g.nodes.find((n) => n.id === g.selectedId) ?? null, [g.nodes, g.selectedId]);
  const view = {
    nodes: g.nodes,
    edges: g.edges,
    positions: g.positions,
    highlights: g.highlights,
    births: g.births,
    remote: g.remote,
    selectedId: g.selectedId,
    version: g.version,
    core,
    lowPower: g.lowPower,
    onSelect: g.select,
  };

  return (
    <div className="relative w-full overflow-hidden rounded-2xl border border-tide/70 bg-[#030814]" style={{ height }}>
      {mode === "3d" ? (
        <Suspense fallback={<NeuralCore2D {...view} />}>
          <NeuralCore3D {...view} />
        </Suspense>
      ) : (
        <NeuralCore2D {...view} />
      )}

      {/* Controles */}
      <div className="absolute top-3 right-3 flex items-center gap-1 rounded-lg bg-abyss/70 p-1 text-xs backdrop-blur">
        {webgl && (
          <>
            <button type="button" aria-pressed={mode === "3d"} onClick={() => g.setMode("3d")} className={`rounded-md px-2 py-1 ${mode === "3d" ? "bg-tide text-ice" : "text-mist hover:text-ice"}`}>
              3D
            </button>
            <button type="button" aria-pressed={mode === "2d"} onClick={() => g.setMode("2d")} className={`rounded-md px-2 py-1 ${mode === "2d" ? "bg-tide text-ice" : "text-mist hover:text-ice"}`}>
              2D
            </button>
          </>
        )}
        <label className="flex cursor-pointer items-center gap-1 px-2 py-1 text-mist">
          <input type="checkbox" checked={g.lowPower} onChange={(e) => g.setLowPower(e.target.checked)} />
          Bajo consumo
        </label>
      </div>

      {/* Leyenda */}
      <ul className="absolute bottom-3 left-3 flex flex-wrap gap-x-3 gap-y-1 rounded-lg bg-abyss/60 px-2 py-1 text-xs text-mist backdrop-blur" aria-label="Tipos de recuerdo">
        {(Object.keys(KIND_LABEL) as (keyof typeof KIND_LABEL)[]).map((k) => (
          <li key={k} className="flex items-center gap-1">
            <span aria-hidden className="inline-block h-2 w-2 rounded-full" style={{ background: KIND_COLOR[k] }} />
            {KIND_LABEL[k]}
          </li>
        ))}
      </ul>

      <p className="absolute top-3 left-3 text-xs text-mist">
        {g.loading && g.nodes.length === 0
          ? "Cargando memoria…"
          : `${g.nodes.length} de ${g.total} recuerdos · ${g.edges.length} relaciones${g.truncated ? " (vista parcial)" : ""}`}
        {!webgl && " · sin WebGL: vista 2D"}
      </p>

      {g.error && <p className="absolute inset-x-0 top-10 text-center text-sm text-coral">{g.error}</p>}
      {!g.loading && !g.error && g.nodes.length === 0 && (
        <p className="absolute inset-x-0 top-1/2 mx-auto max-w-sm -translate-y-1/2 text-center text-sm text-mist">
          Aún no hay recuerdos. Dile a Jarvis «recuerda que…» o añádelos en Memoria; aparecerán aquí.
        </p>
      )}

      {/* Detalle del nodo elegido */}
      {selected && (
        <aside className="pane absolute right-3 bottom-3 w-72 p-4 text-sm" aria-label="Recuerdo seleccionado">
          <div className="mb-1 flex items-center gap-2 text-xs text-mist">
            <span aria-hidden className="inline-block h-2 w-2 rounded-full" style={{ background: KIND_COLOR[selected.kind] }} />
            {KIND_LABEL[selected.kind]} · {VERIFICATION_LABEL[selected.verification]}
          </div>
          <p className="text-ice">{selected.label}</p>
          <p className="mt-2 text-xs text-mist">
            Confianza {Math.round(selected.confidence * 100)} % · {selected.degree} relaciones · {formatDate(selected.createdAt)}
          </p>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              className="text-xs text-cyan hover:underline"
              onClick={() => {
                openMemory(selected.id);
                go("memory");
              }}
            >
              Abrir en Memoria
            </button>
            <button type="button" className="text-xs text-mist hover:text-ice" onClick={() => g.select(null)}>
              Cerrar
            </button>
          </div>
        </aside>
      )}
    </div>
  );
}
