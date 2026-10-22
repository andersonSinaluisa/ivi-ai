// Configuración → Modelos: Qwen en tu servidor (por defecto) o un proveedor propio. En esta PC no corre ningún modelo.
import { useEffect, useState } from "react";
import type { InferenceMode, InferenceSettings, InferenceView } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { useStatus } from "../../stores/statusStore";
import { Button, Dot, Notice } from "../../components/ui";
import { Field, input, Row, SectionTitle } from "./fields";

const MODES: { mode: InferenceMode; label: string; hint: string }[] = [
  { mode: "server", label: "Tu servidor", hint: "Ollama del servidor emparejado, con la credencial de este equipo" },
  { mode: "custom", label: "Proveedor propio", hint: "Otro Ollama o una API compatible, con su URL y su clave" },
];

export function ModelSettings() {
  const status = useStatus((s) => s.status);
  const [view, setView] = useState<InferenceView | null>(null);
  const [draft, setDraft] = useState<InferenceSettings | null>(null);
  const [apiKey, setApiKey] = useState("");
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

  if (!draft || !view) return <p className="text-mist">Cargando…</p>;
  const set = <K extends keyof InferenceSettings>(k: K, v: InferenceSettings[K]) => setDraft({ ...draft, [k]: v });
  const custom = draft.mode === "custom";

  const save = async () => {
    try {
      const v = await backend().updateInference(draft, apiKey === "" ? null : apiKey);
      setView(v);
      setApiKey("");
      setMessage({ tone: "info", text: "Proveedor de modelos guardado." });
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

      <Row label="Ollama del servidor">
        <span className="flex items-center gap-2">
          <Dot ok={Boolean(status?.ollama.ok)} pending={!status} />
          {status?.ollama.detail ?? "sin comprobar"}
        </span>
      </Row>
      <Row label="Qwen">
        <span className="flex items-center gap-2">
          <Dot ok={Boolean(status?.model.ok)} pending={!status} />
          {status ? `${status.model.name}: ${status.model.detail}` : "sin comprobar"}
        </span>
      </Row>
      <Row label="Última respuesta de">{view.status.lastProvider ?? "—"}</Row>

      <Field label="Dónde corre el modelo" group>
        <span className="flex flex-wrap gap-2 pt-0.5" role="radiogroup" aria-label="Dónde corre el modelo">
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
      {custom && (
        <>
          <Field label="Tipo" hint="Ollama u otra API compatible">
            <select className={input} value={draft.remoteKind} onChange={(e) => set("remoteKind", e.target.value as InferenceSettings["remoteKind"])}>
              <option value="ollama">Ollama</option>
              <option value="openai">API compatible (chat completions)</option>
            </select>
          </Field>
          <Field label="URL" hint={draft.remoteKind === "openai" ? "Incluye la versión, p. ej. https://servidor/v1" : "p. ej. https://ollama.tu-dominio"}>
            <input className={input} value={draft.remoteUrl} onChange={(e) => set("remoteUrl", e.target.value)} />
          </Field>
          <Field label="Modelo">
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
        </>
      )}
    </section>
  );
}
