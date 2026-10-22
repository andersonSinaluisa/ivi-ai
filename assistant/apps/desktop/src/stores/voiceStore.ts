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
  handle(m: VoiceServerMessage): void;
}

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
  },

  disconnect() {
    get().client?.close();
    set({ client: null, api: null, connection: "disconnected" });
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
        break;
      default:
        break;
    }
  },
}));
