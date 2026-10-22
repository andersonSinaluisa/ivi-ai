// REST del servicio de voz (siempre en 127.0.0.1, con token).
import type { AudioDevice, VoiceHealth, VoiceProfile } from "@jarvis/shared-types";

export interface VoiceConfig {
  input_device: number | null;
  output_device: number | null;
  volume: number;
  listening_enabled: boolean;
  tts_provider: string;
  tts_speed: number;
  whisper_model: string;
  wake_word_enabled: boolean;
  [key: string]: unknown;
}

export class VoiceApi {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  private async req<T>(path: string, init: RequestInit = {}): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: { "X-Jarvis-Token": this.token, ...(init.headers ?? {}) },
    });
    if (!res.ok) {
      let detail = res.statusText;
      try {
        const body = (await res.json()) as { detail?: string };
        if (body.detail) detail = body.detail;
      } catch {
        /* sin cuerpo JSON */
      }
      throw new Error(detail);
    }
    return (res.status === 204 ? undefined : await res.json()) as T;
  }

  health = () => this.req<VoiceHealth>("/health");
  devices = () => this.req<AudioDevice[]>("/devices");
  config = () => this.req<VoiceConfig>("/config");
  updateConfig = (patch: Partial<VoiceConfig>) =>
    this.req<VoiceConfig>("/config", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(patch),
    });
  voices = () => this.req<VoiceProfile[]>("/voices");
  activateVoice = (id: string | null) =>
    this.req<VoiceProfile[]>("/voices/active", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id }),
    });
  deleteVoice = (id: string) => this.req<void>(`/voices/${encodeURIComponent(id)}`, { method: "DELETE" });

  /** Importa un WAV o usa una grabación previa del micrófono (`recordingId`). */
  createVoice(form: {
    name: string;
    isSelf: boolean;
    consentConfirmed: boolean;
    consentBy: string;
    file?: File;
    recordingId?: string;
  }) {
    const data = new FormData();
    data.set("name", form.name);
    data.set("is_self", String(form.isSelf));
    data.set("consent_confirmed", String(form.consentConfirmed));
    data.set("consent_by", form.consentBy);
    if (form.file) data.set("file", form.file);
    if (form.recordingId) data.set("recording_id", form.recordingId);
    return this.req<VoiceProfile>("/voices", { method: "POST", body: data });
  }

  record = (seconds: number) =>
    this.req<{ recordingId: string; seconds: number }>("/voices/record", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ seconds }),
    });
}
