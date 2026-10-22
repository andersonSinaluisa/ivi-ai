// Configuración de voz: dispositivos, motor de voz y perfiles de voz clonada (con consentimiento).
import { useCallback, useEffect, useState } from "react";
import type { AudioDevice, VoiceProfile } from "@jarvis/shared-types";
import { useVoice } from "../../stores/voiceStore";
import type { VoiceConfig } from "../../services/voiceApi";
import { Button, Notice, ScreenHeader } from "../../components/ui";

const input = "w-full rounded-lg border border-tide bg-deep/70 px-3 py-1.5 text-sm focus:border-cyan focus:outline-none";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="grid grid-cols-[200px_minmax(0,1fr)] items-center gap-4 py-2">
      <span className="text-sm text-ice">{label}</span>
      <span>{children}</span>
    </label>
  );
}

function NewVoiceForm({ onCreated }: { onCreated: () => void }) {
  const api = useVoice((s) => s.api);
  const [name, setName] = useState("");
  const [isSelf, setIsSelf] = useState(true);
  const [consent, setConsent] = useState(false);
  const [consentBy, setConsentBy] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [recordingId, setRecordingId] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const consentOk = isSelf ? consent : consent && consentBy.trim().length > 1;
  const canSave = !!api && name.trim() && consentOk && (file || recordingId) && !busy;

  const record = async () => {
    if (!api) return;
    setBusy("Grabando 10 segundos… lee un texto en voz alta.");
    setError(null);
    try {
      const r = await api.record(10);
      setRecordingId(r.recordingId);
      setFile(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    if (!api) return;
    setBusy("Guardando…");
    setError(null);
    try {
      await api.createVoice({ name, isSelf, consentConfirmed: consent, consentBy, file: file ?? undefined, recordingId: recordingId ?? undefined });
      setName("");
      setFile(null);
      setRecordingId(null);
      setConsent(false);
      onCreated();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="pane mt-4 flex flex-col gap-3 p-4">
      <h3 className="text-sm font-semibold">Nueva voz</h3>
      <input className={input} placeholder="Nombre del perfil" value={name} onChange={(e) => setName(e.target.value)} aria-label="Nombre del perfil de voz" />
      <div className="flex flex-wrap items-center gap-2">
        <Button onClick={() => void record()} disabled={!!busy || !api}>
          Grabar 10 s con el micrófono
        </Button>
        <span className="text-sm text-mist">o</span>
        <input
          type="file"
          accept=".wav,audio/wav"
          aria-label="Importar WAV"
          onChange={(e) => {
            setFile(e.target.files?.[0] ?? null);
            setRecordingId(null);
          }}
          className="text-sm text-mist file:mr-2 file:rounded-lg file:border file:border-tide file:bg-deep file:px-3 file:py-1.5 file:text-ice"
        />
      </div>
      {recordingId && <p className="text-sm text-kelp">Grabación lista.</p>}
      <fieldset className="flex flex-col gap-2 text-sm">
        <legend className="mb-1 text-ice">¿De quién es esta voz?</legend>
        <label className="flex items-center gap-2">
          <input type="radio" checked={isSelf} onChange={() => setIsSelf(true)} /> Es mi propia voz
        </label>
        <label className="flex items-center gap-2">
          <input type="radio" checked={!isSelf} onChange={() => setIsSelf(false)} /> Es la voz de otra persona
        </label>
        {!isSelf && (
          <input className={input} placeholder="Nombre de la persona que dio su consentimiento" value={consentBy} onChange={(e) => setConsentBy(e.target.value)} aria-label="Persona que consiente" />
        )}
        <label className="flex items-start gap-2">
          <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} className="mt-1" />
          <span>
            {isSelf
              ? "Confirmo que la grabación es de mi voz y acepto que se guarde en esta PC."
              : "Confirmo que esta persona me dio su consentimiento explícito para clonar su voz."}{" "}
            La muestra se guarda solo en esta PC y no se envía a ningún servicio externo.
          </span>
        </label>
      </fieldset>
      {busy && <Notice>{busy}</Notice>}
      {error && <Notice tone="error">{error}</Notice>}
      <div>
        <Button tone="primary" onClick={() => void save()} disabled={!canSave}>
          Guardar voz
        </Button>
      </div>
    </div>
  );
}

/** Dónde se transcribe la voz: en esta PC o en el servidor (jarvis-stt del VPS). */
export function TranscriptionSettings({ config, onChange }: { config: VoiceConfig; onChange: (c: VoiceConfig) => void }) {
  const api = useVoice((s) => s.api);
  const [url, setUrl] = useState(config.stt_remote_url);
  const [token, setToken] = useState("");
  const [message, setMessage] = useState<{ tone: "info" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const remote = config.stt_provider === "remote";

  const run = async (action: () => Promise<string | void>) => {
    if (!api) return;
    setBusy(true);
    setMessage(null);
    try {
      const text = await action();
      if (text) setMessage({ tone: "info", text });
    } catch (e) {
      setMessage({ tone: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      if (!api) return;
      let next = await api.updateConfig({ stt_provider: "remote", stt_remote_url: url.trim() });
      if (token.trim()) next = await api.setRemoteSttToken(token.trim());
      onChange(next);
      setToken("");
      const check = await api.checkStt();
      if (!check.ok) throw new Error(check.detail);
      return `Servidor listo: ${check.detail}`;
    });

  return (
    <div className="mt-2 rounded-xl border border-tide p-3" role="group" aria-label="Transcripción">
      <div className="flex items-center gap-3 py-1">
        <span className="w-[200px] text-sm text-ice">Transcripción</span>
        <span className="flex gap-2" role="radiogroup" aria-label="Dónde se transcribe">
          {[
            { id: "faster-whisper", label: "En esta PC" },
            { id: "remote", label: "En el servidor" },
          ].map((o) => (
            <button
              key={o.id}
              type="button"
              role="radio"
              aria-checked={config.stt_provider === o.id}
              disabled={busy || (o.id === "remote" && !config.stt_remote_url)}
              onClick={() => void run(async () => {
                if (!api) return;
                onChange(await api.updateConfig({ stt_provider: o.id }));
                return o.id === "remote" ? "Se transcribe en el servidor." : "Se transcribe en esta PC.";
              })}
              className={`rounded-lg border px-3 py-1.5 text-sm ${config.stt_provider === o.id ? "border-cyan bg-cyan/10 text-ice" : "border-tide text-mist hover:text-ice"} disabled:opacity-40`}
            >
              {o.label}
            </button>
          ))}
        </span>
      </div>
      <p className="pb-2 text-xs text-mist">
        El micrófono y la detección de frases siguen en esta PC; con «En el servidor» solo se envía el audio de cada frase por HTTPS.
        {remote && " Si el servidor no responde y hay un modelo copiado en models\\whisper, se usa el local."}
      </p>
      <Row label="URL del servidor">
        <input className={input} placeholder="https://jarvis.tu-dominio/stt" value={url} onChange={(e) => setUrl(e.target.value)} />
      </Row>
      <Row label="Token">
        <input
          className={input}
          type="password"
          autoComplete="off"
          placeholder={config.stt_remote_token_set ? "•••••• (guardado)" : "el de /etc/nginx/jarvis-extra/stt.conf"}
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
      </Row>
      {message && <Notice tone={message.tone}>{message.text}</Notice>}
      <div className="mt-2 flex gap-2">
        <Button tone="primary" disabled={busy || !url.trim()} onClick={() => void save()}>
          Guardar y usar el servidor
        </Button>
        <Button disabled={busy} onClick={() => void run(async () => {
          if (!api) return;
          const check = await api.checkStt();
          if (!check.ok) throw new Error(check.detail);
          return check.detail;
        })}>
          Probar
        </Button>
      </div>
    </div>
  );
}

export function VoiceSettings() {
  const { api, connection, unavailableReason } = useVoice();
  const [config, setConfig] = useState<VoiceConfig | null>(null);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [voices, setVoices] = useState<VoiceProfile[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    if (!api) return;
    api.config().then(setConfig).catch((e: Error) => setError(e.message));
    api.devices().then(setDevices).catch(() => setDevices([]));
    api.voices().then(setVoices).catch(() => setVoices([]));
  }, [api]);
  useEffect(load, [load]);

  const update = async (patch: Partial<VoiceConfig>) => {
    if (!api) return;
    try {
      setConfig(await api.updateConfig(patch));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  if (connection !== "open" || !api) {
    return (
      <section>
        <ScreenHeader title="Voz" />
        <Notice tone="warn">{unavailableReason ?? "El servicio de voz no está conectado. Inícialo con scripts\\start-voice.ps1."}</Notice>
      </section>
    );
  }

  return (
    <section>
      <ScreenHeader title="Voz" />
      {error && <Notice tone="error">{error}</Notice>}
      {config && (
        <>
          <Row label="Micrófono">
            <select className={input} value={config.input_device ?? ""} onChange={(e) => void update({ input_device: e.target.value === "" ? null : Number(e.target.value) })}>
              <option value="">Predeterminado del sistema</option>
              {devices.filter((d) => d.inputs > 0).map((d) => (
                <option key={d.index} value={d.index}>{d.name}</option>
              ))}
            </select>
          </Row>
          <Row label="Altavoz">
            <select className={input} value={config.output_device ?? ""} onChange={(e) => void update({ output_device: e.target.value === "" ? null : Number(e.target.value) })}>
              <option value="">Predeterminado del sistema</option>
              {devices.filter((d) => d.outputs > 0).map((d) => (
                <option key={d.index} value={d.index}>{d.name}</option>
              ))}
            </select>
          </Row>
          <Row label={`Volumen (${Math.round(config.volume * 100)} %)`}>
            <input type="range" min={0} max={1} step={0.05} value={config.volume} onChange={(e) => void update({ volume: Number(e.target.value) })} className="w-full accent-cyan" />
          </Row>
          <Row label="Motor de voz">
            <select className={input} value={config.tts_provider} onChange={(e) => void update({ tts_provider: e.target.value })}>
              <option value="sapi">Voz de Windows (sin clonación)</option>
              <option value="openvoice">OpenVoice V2 (voz clonada)</option>
            </select>
          </Row>
          <Row label="Velocidad">
            <input type="number" min={0.5} max={2} step={0.1} className={input} value={config.tts_speed} onChange={(e) => void update({ tts_speed: Number(e.target.value) })} />
          </Row>
          <TranscriptionSettings config={config} onChange={setConfig} />
          <Row label="Modelo de transcripción">
            <select className={input} value={config.whisper_model} onChange={(e) => void update({ whisper_model: e.target.value })}>
              {["tiny", "base", "small", "medium"].map((m) => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
          </Row>
        </>
      )}

      <h3 className="mt-6 mb-2 text-sm font-semibold">Voces guardadas</h3>
      {voices.length === 0 ? (
        <p className="text-sm text-mist">No hay voces guardadas. Jarvis usa la voz de Windows.</p>
      ) : (
        <ul className="divide-y divide-tide/60">
          {voices.map((v) => (
            <li key={v.id} className="flex items-center gap-3 py-2 text-sm">
              <span className="text-ice">{v.name}</span>
              <span className="text-xs text-mist">{v.isSelf ? "Voz propia" : `Consentimiento de ${v.consentBy}`}</span>
              {v.active ? (
                <span className="ml-auto text-xs text-cyan">Activa</span>
              ) : (
                <Button className="ml-auto" onClick={() => void api.activateVoice(v.id).then(setVoices)}>Usar</Button>
              )}
              <Button tone="danger" onClick={() => void api.deleteVoice(v.id).then(load)}>Eliminar</Button>
            </li>
          ))}
        </ul>
      )}
      <NewVoiceForm onCreated={load} />
    </section>
  );
}
