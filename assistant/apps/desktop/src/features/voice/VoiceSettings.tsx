// Configuración de voz: micrófono y altavoz de esta PC; la transcripción y la voz las hace tu servidor.
import { useCallback, useEffect, useState } from "react";
import type { AudioDevice, VoiceHealth } from "@jarvis/shared-types";
import { useVoice } from "../../stores/voiceStore";
import type { VoiceConfig } from "../../services/voiceApi";
import { Button, Dot, Notice, ScreenHeader } from "../../components/ui";

const input = "w-full rounded-lg border border-tide bg-deep/70 px-3 py-1.5 text-sm focus:border-cyan focus:outline-none";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="grid grid-cols-[200px_minmax(0,1fr)] items-center gap-4 py-2">
      <span className="text-sm text-ice">{label}</span>
      <span>{children}</span>
    </label>
  );
}

function Status({ label, ok, detail }: { label: string; ok: boolean; detail: string }) {
  return (
    <div className="grid grid-cols-[200px_minmax(0,1fr)] items-baseline gap-4 py-1.5 text-sm">
      <span className="text-mist">{label}</span>
      <span className="flex items-center gap-2 text-ice">
        <Dot ok={ok} />
        {detail}
      </span>
    </div>
  );
}

export function VoiceSettings() {
  const { api, connection, unavailableReason, serverError } = useVoice();
  const [config, setConfig] = useState<VoiceConfig | null>(null);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [health, setHealth] = useState<VoiceHealth | null>(null);
  const [message, setMessage] = useState<{ tone: "info" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    if (!api) return;
    api.config().then(setConfig).catch((e: Error) => setMessage({ tone: "error", text: e.message }));
    api.devices().then(setDevices).catch(() => setDevices([]));
    api.health().then(setHealth).catch(() => setHealth(null));
  }, [api]);
  useEffect(load, [load]);

  const update = async (patch: Partial<VoiceConfig>) => {
    if (!api) return;
    try {
      setConfig(await api.updateConfig(patch));
      setMessage(null);
    } catch (e) {
      setMessage({ tone: "error", text: e instanceof Error ? e.message : String(e) });
    }
  };

  const check = async () => {
    if (!api) return;
    setBusy(true);
    try {
      const r = await api.check();
      setMessage(r.ok ? { tone: "info", text: "Transcripción y voz responden en tu servidor." } : { tone: "error", text: [r.stt, r.tts].filter((x) => !x.ok).map((x) => x.detail).join(" · ") });
      setHealth(await api.health());
    } catch (e) {
      setMessage({ tone: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
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
    <section aria-label="Voz">
      <ScreenHeader title="Voz">
        <Button disabled={busy} onClick={() => void check()}>
          Probar
        </Button>
      </ScreenHeader>
      {message && <Notice tone={message.tone}>{message.text}</Notice>}
      {serverError && <Notice tone="warn">{serverError}</Notice>}
      <p className="pb-2 text-sm text-mist">
        El micrófono, la detección de frases y el altavoz están en esta PC. La transcripción (Whisper) y la voz (Piper) se hacen en tu
        servidor con la credencial de este equipo; sin conexión no hay voz.
      </p>
      {health && (
        <>
          <Status label="Servidor" ok={health.server.connected} detail={health.server.url ?? "sin conectar (empareja la PC en Infraestructura)"} />
          <Status label="Transcripción" ok={health.stt.ready} detail={health.stt.detail} />
          <Status label="Voz" ok={health.tts.ready} detail={health.tts.detail} />
        </>
      )}
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
          <Row label="Voz de Jarvis">
            <select className={input} value={config.tts_provider} onChange={(e) => void update({ tts_provider: e.target.value })}>
              <option value="server">La de tu servidor</option>
              <option value="sapi">Voz de Windows</option>
            </select>
          </Row>
          <Row label="Velocidad">
            <input type="number" min={0.5} max={2} step={0.1} className={input} value={config.tts_speed} onChange={(e) => void update({ tts_speed: Number(e.target.value) })} />
          </Row>
        </>
      )}
    </section>
  );
}
