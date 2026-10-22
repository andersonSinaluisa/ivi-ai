import { create } from "zustand";
import type { VoiceServerMessage, VoiceState, WakeWordStatus } from "@jarvis/shared-types";
import { backend } from "../services/backend";
import { type ConnectionState, VoiceClient } from "../services/voiceClient";
import { VoiceApi } from "../services/voiceApi";

interface VoiceStore {
  connection: ConnectionState;
  unavailableReason: string | null;
  state: VoiceState;
  listening: boolean;
  wakeWord: WakeWordStatus | null;
  level: number;
  /** Amplitud del audio que está reproduciendo Jarvis (0..1). */
  outputLevel: number;
  lastTranscript: string | null;
  lastError: string | null;
  /** Por qué el servicio de voz no puede usar el servidor (sin emparejar, sin red…). */
  serverError: string | null;
  api: VoiceApi | null;
  client: VoiceClient | null;
  onTranscript: ((text: string) => void) | null;
  connect(): Promise<void>;
  disconnect(): void;
  pttStart(): void;
  pttStop(): void;
  cancel(): void;
  speak(text: string): void;
  agentDone(): void;
  thinking(): void;
  stopSpeaking(): void;
  setListening(enabled: boolean): void;
  /** Entrega al servicio de voz la URL del servidor y el token del equipo (y lo renueva). */
  pushServerCredential(): Promise<void>;
  handle(m: VoiceServerMessage): void;
}

/** El token del device dura 10 min y Rust lo renueva con 3 min de margen: cada minuto basta. */
export const CREDENTIAL_REFRESH_MS = 60_000;
let credentialTimer: ReturnType<typeof setInterval> | null = null;

export const useVoice = create<VoiceStore>((set, get) => ({
  connection: "disconnected",
  unavailableReason: null,
  state: "IDLE",
  listening: false,
  wakeWord: null,
  level: 0,
  outputLevel: 0,
  lastTranscript: null,
  lastError: null,
  serverError: null,
  api: null,
  client: null,
  onTranscript: null,

  async connect() {
    if (get().client) return;
    let conn;
    try {
      conn = await backend().voiceConnection();
    } catch (e) {
      set({ unavailableReason: e instanceof Error ? e.message : String(e) });
      return;
    }
    if (!conn.token) {
      set({ unavailableReason: conn.error ?? "Servicio de voz no disponible" });
      return;
    }
    const wsUrl = conn.baseUrl.replace(/^http/, "ws") + "/ws";
    const client = new VoiceClient({
      url: wsUrl,
      token: conn.token,
      onMessage: (m) => get().handle(m),
      onConnection: (c) =>
        set({
          connection: c,
          unavailableReason:
            c === "unauthorized" ? "El token del servicio de voz no coincide. Reinicia el servicio y la app." : null,
        }),
    });
    set({ client, api: new VoiceApi(conn.baseUrl, conn.token), unavailableReason: null });
    client.connect();
    void get().pushServerCredential();
    if (credentialTimer) clearInterval(credentialTimer);
    credentialTimer = setInterval(() => void get().pushServerCredential(), CREDENTIAL_REFRESH_MS);
  },

  disconnect() {
    get().client?.close();
    if (credentialTimer) clearInterval(credentialTimer);
    credentialTimer = null;
    set({ client: null, api: null, connection: "disconnected" });
  },

  async pushServerCredential() {
    const api = get().api;
    if (!api) return;
    try {
      const credential = await backend().cloudServiceCredential();
      await api.setServer(credential.serverUrl, credential.token);
      set({ serverError: null });
    } catch (e) {
      set({ serverError: e instanceof Error ? e.message : String(e) });
    }
  },

  pttStart: () => void get().client?.send({ type: "ptt_start" }),
  pttStop: () => void get().client?.send({ type: "ptt_stop" }),
  cancel: () => void get().client?.send({ type: "cancel" }),
  speak: (text) => void get().client?.send({ type: "speak", text }),
  agentDone: () => void get().client?.send({ type: "agent_done" }),
  thinking: () => void get().client?.send({ type: "thinking" }),
  stopSpeaking: () => void get().client?.send({ type: "stop_speaking" }),
  setListening: (enabled) => void get().client?.send({ type: "set_listening", enabled }),

  handle(m) {
    switch (m.type) {
      case "hello":
        set({ state: m.state, wakeWord: m.wakeWord, listening: m.listening, lastError: null });
        break;
      case "state":
        set({ state: m.state, level: m.state === "RECORDING_COMMAND" ? get().level : 0, outputLevel: m.state === "SPEAKING" ? get().outputLevel : 0 });
        break;
      case "output_level":
        set({ outputLevel: m.rms });
        break;
      case "level":
        set({ level: m.rms });
        break;
      case "transcript":
        set({ lastTranscript: m.text });
        get().onTranscript?.(m.text);
        break;
      case "info":
        if (m.code === "listening") set({ listening: m.message === "on" });
        break;
      case "error":
        set({ lastError: m.message });
        // La credencial caducó o se revocó: pide una nueva enseguida (Rust la renueva si hace falta).
        if (m.code === "server_auth") void get().pushServerCredential();
        break;
      default:
        break;
    }
  },
}));
