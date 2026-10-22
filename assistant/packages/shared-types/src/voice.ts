// Protocolo WebSocket del servicio de voz (services/voice-service/app/api/ws.py).

export type VoiceState =
  | "IDLE"
  | "LISTENING_WAKE_WORD"
  | "WAKE_DETECTED"
  | "RECORDING_COMMAND"
  | "TRANSCRIBING"
  | "THINKING"
  | "SPEAKING"
  | "ERROR";

/** Mensajes servidor → cliente. */
export type VoiceServerMessage =
  | { type: "hello"; state: VoiceState; wakeWord: WakeWordStatus; listening: boolean }
  | { type: "state"; state: VoiceState; previous: VoiceState; reason?: string | null }
  | { type: "level"; rms: number; peak: number }
  /** Amplitud del audio que reproduce Jarvis, para sincronizar el núcleo visual. */
  | { type: "output_level"; rms: number }
  | { type: "wake"; score: number }
  | { type: "transcript"; text: string; language: string; durationMs: number }
  | { type: "speaking"; chunk: number; total: number }
  | { type: "info"; code: string; message: string }
  | { type: "error"; code: string; message: string };

/** Mensajes cliente → servidor. El primero debe ser `auth`. */
export type VoiceClientMessage =
  | { type: "auth"; token: string }
  | { type: "ptt_start" }
  | { type: "ptt_stop" }
  | { type: "cancel" }
  | { type: "thinking" }
  | { type: "agent_done" }
  | { type: "speak"; text: string }
  | { type: "stop_speaking" }
  | { type: "set_listening"; enabled: boolean };

export interface WakeWordStatus {
  enabled: boolean;
  available: boolean;
  model: string | null;
  detail: string;
}

export interface VoiceHealth {
  status: "ok";
  state: VoiceState;
  stt: ComponentStatus;
  tts: ComponentStatus;
  audio: ComponentStatus;
  wakeWord: WakeWordStatus;
  /** Motores opcionales (p. ej. VibeVoice, experimental). */
  engines?: { vibevoice?: VibeVoiceStatus };
}

export interface VibeVoiceStatus {
  configured: boolean;
  packageInstalled: boolean;
  torchInstalled: boolean;
  detail: string;
  license: string;
}

export interface ComponentStatus {
  ready: boolean;
  provider: string;
  detail: string;
}

export interface AudioDevice {
  index: number;
  name: string;
  inputs: number;
  outputs: number;
  isDefaultInput: boolean;
  isDefaultOutput: boolean;
}

export interface VoiceProfile {
  id: string;
  name: string;
  isSelf: boolean;
  consentConfirmed: boolean;
  consentBy: string | null;
  createdAt: string;
  active: boolean;
}

export interface VoiceConnection {
  baseUrl: string;
  token: string | null;
  error: string | null;
}
