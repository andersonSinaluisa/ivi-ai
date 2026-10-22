// REST del servicio de voz (siempre en 127.0.0.1, con token).
import type { AudioDevice, VoiceHealth, VoiceServerCheck } from "@jarvis/shared-types";

export interface VoiceConfig {
  input_device: number | null;
  output_device: number | null;
  volume: number;
  listening_enabled: boolean;
  /** "server" (voz de tu servidor, por defecto), "sapi" (voz de Windows) o "silent". */
  tts_provider: string;
  tts_speed: number;
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
  /** Entrega al servicio de voz la URL del servidor y el token del equipo (solo en memoria; se renueva cada minuto). */
  setServer = (serverUrl: string, token: string) =>
    this.req<{ connected: boolean; url: string; check: Omit<VoiceServerCheck, "ok"> | null }>("/cloud", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ server_url: serverUrl, token }),
    });
  clearServer = () => this.req<void>("/cloud", { method: "DELETE" });
  /** Comprueba transcripción y voz en el servidor. */
  check = () => this.req<VoiceServerCheck>("/check", { method: "POST" });
}
