import { useState } from "react";
import { PHASES, PHASE_LABEL } from "../../components/phases";
import { Dot } from "../../components/ui";
import { useAssistant } from "../../stores/assistantStore";
import { useStatus } from "../../stores/statusStore";
import { useUi } from "../../stores/uiStore";
import { useVoice } from "../../stores/voiceStore";
import { VoiceDock, VOICE_LABEL } from "../voice/VoiceDock";
import { NeuralCore } from "../neural/NeuralCore";

export function DashboardScreen() {
  const run = useAssistant((s) => s.activeRun);
  const lastPhases = useAssistant((s) => s.lastPhases);
  const awaiting = useAssistant((s) => s.messages.some((m) => m.steps.some((st) => st.status === "awaiting")));
  const send = useAssistant((s) => s.send);
  const status = useStatus((s) => s.status);
  const voice = useVoice();
  const go = useUi((s) => s.go);
  const [text, setText] = useState("");

  const seen = run?.phasesSeen ?? lastPhases;
  const headline = run
    ? awaiting
      ? "Espero tu autorización"
      : run.phase
        ? `${PHASE_LABEL[run.phase]}…`
        : "Empezando…"
    : voice.connection === "open" && voice.state !== "IDLE"
      ? VOICE_LABEL[voice.state]
      : "¿En qué te ayudo?";

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    void send(text);
    setText("");
    go("chat");
  };

  const services = status
    ? [
        { name: "Servidor (Ollama)", ok: status.ollama.ok, detail: status.ollama.detail },
        { name: `Modelo ${status.model.name}`, ok: status.model.ok, detail: status.model.detail },
        { name: "windows-agent", ok: status.windowsAgent.ok, detail: status.windowsAgent.ok ? `${status.windowsAgent.toolCount} herramientas` : status.windowsAgent.detail },
        { name: "Voz", ok: voice.connection === "open", detail: voice.connection === "open" ? VOICE_LABEL[voice.state] : (voice.unavailableReason ?? "sin conexión") },
        { name: "Base local", ok: status.database.ok, detail: status.database.detail },
      ]
    : [];

  return (
    <div className="grid min-h-full grid-cols-[minmax(0,1fr)_300px] gap-6">
      <section className="flex min-w-0 flex-col items-center gap-4">
        <NeuralCore height={480} />
        <p className="font-display text-3xl font-light tracking-tight text-ice" aria-live="polite">
          {headline}
        </p>
        <ol className="flex flex-wrap justify-center gap-x-3 gap-y-1 text-xs text-mist" aria-label="Ciclo del agente">
          {PHASES.map((p) => (
            <li key={p} className={run?.phase === p ? "text-cyan" : seen.includes(p) ? "text-ice/80" : ""}>
              {PHASE_LABEL[p]}
            </li>
          ))}
        </ol>
        <form onSubmit={submit} className="flex w-full max-w-xl gap-2">
          <input
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Escribe una instrucción, por ejemplo: recuerda que…"
            aria-label="Instrucción para Jarvis"
            disabled={run !== null}
            className="flex-1 rounded-xl border border-tide bg-deep/70 px-4 py-3 text-ice placeholder:text-mist/70 focus:border-cyan focus:outline-none"
          />
        </form>
        <VoiceDock compact />
      </section>

      <aside className="pane flex flex-col gap-1 self-start p-5">
        <h2 className="mb-2 text-sm font-semibold text-ice">Servicios</h2>
        {services.length === 0 && <p className="text-sm text-mist">Comprobando…</p>}
        {services.map((s) => (
          <div key={s.name} className="flex gap-3 py-2">
            <span className="pt-1.5">
              <Dot ok={s.ok} />
            </span>
            <div className="min-w-0">
              <p className="text-sm text-ice">{s.name}</p>
              <p className="text-xs break-words text-mist">{s.detail}</p>
            </div>
          </div>
        ))}
        {status && !status.model.ok && status.model.alternatives.length > 0 && (
          <button type="button" onClick={() => go("settings")} className="mt-2 text-left text-sm text-cyan hover:underline">
            Elegir otro modelo Qwen instalado
          </button>
        )}
      </aside>
    </div>
  );
}
