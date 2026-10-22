// Configuración → Modelos: Qwen local (Ollama), proveedor remoto del Inference Gateway, preferencia de inferencia
// y estado de VibeVoice (opcional, experimental).
import { useEffect, useState } from "react";
import type { InferenceMode, InferenceSettings, InferenceView, VibeVoiceStatus } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { useStatus } from "../../stores/statusStore";
import { useVoice } from "../../stores/voiceStore";
import { Button, Dot, Notice } from "../../components/ui";
import { Field, input, Row, SectionTitle } from "./fields";

const MODES: { mode: InferenceMode; label: string; hint: string }[] = [
  { mode: "local", label: "Local", hint: "Solo Ollama en esta PC" },
  { mode: "auto", label: "Automático", hint: "Local y, si no responde, el remoto" },
  { mode: "remote", label: "Remoto", hint: "Siempre el proveedor remoto" },
];

export function ModelSettings() {
  const status = useStatus((s) => s.status);
  const api = useVoice((s) => s.api);
  const [view, setView] = useState<InferenceView | null>(null);
  const [draft, setDraft] = useState<InferenceSettings | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [vibevoice, setVibevoice] = useState<VibeVoiceStatus | null>(null);
  const [message, setMessage] = useState<{ tone: "info" | "error"; text: string } | null>(null);

  useEffect(() => {
    void backend()
      .getInference()
      .then((v) => {
        setView(v);
        setDraft(v.settings);
      })
      .catch((e: unknown) => setMessage({ tone: "error", text: e instanceof Error ? e.message : String(e) }));
  }, []);
  useEffect(() => {
    if (!api) return;
    void api
      .health()
      .then((h) => setVibevoice(h.engines?.vibevoice ?? null))
      .catch(() => setVibevoice(null));
  }, [api]);

  if (!draft || !view) return <p className="text-mist">Cargando…</p>;
  const set = <K extends keyof InferenceSettings>(k: K, v: InferenceSettings[K]) => setDraft({ ...draft, [k]: v });

  const save = async () => {
    try {
      const v = await backend().updateInference(draft, apiKey === "" ? null : apiKey);
      setView(v);
      setApiKey("");
      setMessage({ tone: "info", text: "Proveedor de inferencia guardado." });
    } catch (e) {
      setMessage({ tone: "error", text: e instanceof Error ? e.message : String(e) });
    }
  };
  const test = async () => {
    const r = await backend().testInference();
    setMessage({ tone: r.ok ? "info" : "error", text: r.ok ? `Responde: ${r.detail}` : r.detail });
  };

  return (
    <section aria-label="Modelos">
      <SectionTitle title="Modelos">
        <Button onClick={() => void test()}>Probar</Button>
        <Button tone="primary" onClick={() => void save()}>
          Guardar
        </Button>
      </SectionTitle>
      {message && <Notice tone={message.tone}>{message.text}</Notice>}

      <Row label="Qwen local">
        <span className="flex items-center gap-2">
          <Dot ok={Boolean(status?.model.ok)} pending={!status} />
          {status ? `${status.model.name}: ${status.model.detail}` : "sin comprobar"}
        </span>
      </Row>
      <Row label="Estado de Ollama">
        <span className="flex items-center gap-2">
          <Dot ok={Boolean(status?.ollama.ok)} pending={!status} />
          {status?.ollama.detail ?? "sin comprobar"}
        </span>
      </Row>
      <Row label="Última respuesta de">{view.status.lastProvider ?? "—"}{view.status.lastFallback && <span className="ml-2 text-amber">(el local no respondió)</span>}</Row>

      <Field label="Preferencia de inferencia" group hint="El agente no cambia: solo dónde corre el modelo">
        <span className="flex flex-wrap gap-2 pt-0.5" role="radiogroup" aria-label="Preferencia de inferencia">
          {MODES.map((m) => (
            <button
              key={m.mode}
              type="button"
              role="radio"
              aria-checked={draft.mode === m.mode}
              title={m.hint}
              onClick={() => set("mode", m.mode)}
              className={`rounded-lg border px-3 py-1.5 text-sm ${draft.mode === m.mode ? "border-cyan bg-cyan/10 text-ice" : "border-tide text-mist hover:text-ice"}`}
            >
              {m.label}
            </button>
          ))}
        </span>
      </Field>
      <Field label="Proveedor remoto" hint="Ollama en otro equipo o una API compatible">
        <select className={input} value={draft.remoteKind} onChange={(e) => set("remoteKind", e.target.value as InferenceSettings["remoteKind"])}>
          <option value="ollama">Ollama remoto</option>
          <option value="openai">API compatible (chat completions)</option>
        </select>
      </Field>
      <Field label="URL" hint={draft.remoteKind === "openai" ? "Incluye la versión, p. ej. https://servidor/v1" : "p. ej. https://ollama.tu-dominio"}>
        <input className={input} value={draft.remoteUrl} onChange={(e) => set("remoteUrl", e.target.value)} />
      </Field>
      <Field label="Modelo remoto">
        <input className={input} value={draft.remoteModel} onChange={(e) => set("remoteModel", e.target.value)} />
      </Field>
      <Field label="Clave" hint={`Se guarda en ${view.vault}; nunca en archivos de configuración`}>
        <span className="flex gap-2">
          <input
            className={input}
            type="password"
            autoComplete="off"
            placeholder={view.hasApiKey ? "•••••• (guardada)" : "opcional"}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
          />
          {view.hasApiKey && (
            <Button
              tone="danger"
              onClick={() =>
                void backend()
                  .updateInference(draft, "")
                  .then((v) => {
                    setView(v);
                    setMessage({ tone: "info", text: "Clave borrada." });
                  })
              }
            >
              Borrar
            </Button>
          )}
        </span>
      </Field>

      <Row label="VibeVoice">
        <span className="flex items-center gap-2">
          <Dot ok={Boolean(vibevoice?.configured)} pending={!vibevoice} />
          {vibevoice ? vibevoice.detail : "servicio de voz no disponible"}
        </span>
      </Row>
      {vibevoice && (
        <p className="pl-[216px] text-xs text-mist">
          Opcional y experimental: {vibevoice.license}. La voz principal sigue siendo la de Windows.
        </p>
      )}
    </section>
  );
}
