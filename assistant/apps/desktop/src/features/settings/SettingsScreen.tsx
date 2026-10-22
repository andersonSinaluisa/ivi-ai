import { useEffect, useState } from "react";
import type { RiskClass, Settings } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { useStatus } from "../../stores/statusStore";
import { Button, Notice, RISK_LABEL, ScreenHeader } from "../../components/ui";
import { VoiceSettings } from "../voice/VoiceSettings";
import { CloudSettings } from "./CloudSettings";
import { ModelSettings } from "./ModelSettings";
import { Field, input } from "./fields";

const RISKS: RiskClass[] = ["READ_ONLY", "LOW_RISK", "SENSITIVE"];

export function SettingsScreen() {
  const { settings, loadSettings, saveSettings, refresh } = useStatus();
  const [draft, setDraft] = useState<Settings | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [message, setMessage] = useState<{ tone: "info" | "error"; text: string } | null>(null);

  useEffect(() => {
    void loadSettings();
    void backend().listModels().then(setModels).catch(() => setModels([]));
  }, [loadSettings]);
  useEffect(() => setDraft(settings), [settings]);

  if (!draft) return <p className="text-mist">Cargando…</p>;
  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => setDraft({ ...draft, [k]: v });

  const save = async () => {
    try {
      await saveSettings(draft);
      await refresh();
      setMessage({ tone: "info", text: "Configuración guardada." });
    } catch (e) {
      setMessage({ tone: "error", text: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-10">
      <section>
        <ScreenHeader title="Modelo">
          <Button tone="primary" onClick={() => void save()}>
            Guardar
          </Button>
        </ScreenHeader>
        {message && <Notice tone={message.tone}>{message.text}</Notice>}
        <Field label="Modelo de Ollama" hint="Debe admitir herramientas (Qwen3 lo hace)">
          <input list="models" className={input} value={draft.model} onChange={(e) => set("model", e.target.value)} />
          <datalist id="models">
            {models.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
          {models.length > 0 && <span className="mt-1 block text-xs text-mist">Instalados: {models.join(", ")}</span>}
        </Field>
        <Field label="URL de Ollama">
          <input className={input} value={draft.baseUrl} onChange={(e) => set("baseUrl", e.target.value)} />
        </Field>
        <Field label="Temperatura" hint="Más baja = más predecible">
          <input type="number" step={0.1} min={0} max={2} className={input} value={draft.temperature} onChange={(e) => set("temperature", Number(e.target.value))} />
        </Field>
        <Field label="Contexto (tokens)" hint="Más contexto usa más memoria de GPU">
          <input type="number" step={1024} min={1024} className={input} value={draft.numCtx} onChange={(e) => set("numCtx", Number(e.target.value))} />
        </Field>
        <Field label="Razonamiento previo" hint="Más lento, a veces más preciso">
          <input type="checkbox" checked={draft.think} onChange={(e) => set("think", e.target.checked)} className="mt-2" />
        </Field>
        <Field label="Pasos máximos" hint="Vueltas modelo → herramienta por instrucción">
          <input type="number" min={1} max={20} className={input} value={draft.maxSteps} onChange={(e) => set("maxSteps", Number(e.target.value))} />
        </Field>
        <Field label="Ejecutar sin preguntar" hint="Las destructivas siempre preguntan">
          <span className="flex flex-col gap-1 pt-1">
            {RISKS.map((r) => (
              <span key={r} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  id={`auto-${r}`}
                  checked={draft.autoApprove.includes(r)}
                  onChange={(e) =>
                    set("autoApprove", e.target.checked ? [...draft.autoApprove, r] : draft.autoApprove.filter((x) => x !== r))
                  }
                />
                <label htmlFor={`auto-${r}`}>{RISK_LABEL[r]}</label>
              </span>
            ))}
          </span>
        </Field>
      </section>
      <ModelSettings />
      <CloudSettings />
      <VoiceSettings />
    </div>
  );
}
