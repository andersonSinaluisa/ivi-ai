// Cliente WebSocket del servicio de voz, con autenticación por primer mensaje y reconexión con
// espera exponencial.
import type { VoiceClientMessage, VoiceServerMessage } from "@jarvis/shared-types";

export type ConnectionState = "disconnected" | "connecting" | "open" | "unauthorized";

/** Espera antes del intento `attempt` (0, 1, 2…): 0.5 s, 1 s, 2 s… hasta 15 s. */
export function backoffMs(attempt: number): number {
  return Math.min(15_000, 500 * 2 ** Math.max(0, attempt));
}

export interface VoiceClientOptions {
  url: string; // ws://127.0.0.1:8765/ws
  token: string;
  onMessage: (m: VoiceServerMessage) => void;
  onConnection: (s: ConnectionState) => void;
  /** Inyectable en pruebas. */
  createSocket?: (url: string) => WebSocket;
  schedule?: (fn: () => void, ms: number) => unknown;
}

/** Código de cierre que usa el servidor cuando el token es inválido. */
export const CLOSE_UNAUTHORIZED = 4401;

export class VoiceClient {
  private socket: WebSocket | null = null;
  private attempt = 0;
  private stopped = false;
  private readonly opts: Required<VoiceClientOptions>;

  constructor(opts: VoiceClientOptions) {
    this.opts = {
      createSocket: (url) => new WebSocket(url),
      schedule: (fn, ms) => setTimeout(fn, ms),
      ...opts,
    };
  }

  connect(): void {
    this.stopped = false;
    this.open();
  }

  private open(): void {
    if (this.stopped) return;
    this.opts.onConnection("connecting");
    const ws = this.opts.createSocket(this.opts.url);
    this.socket = ws;
    ws.onopen = () => {
      ws.send(JSON.stringify({ type: "auth", token: this.opts.token } satisfies VoiceClientMessage));
    };
    ws.onmessage = (ev) => {
      let msg: VoiceServerMessage;
      try {
        msg = JSON.parse(String(ev.data)) as VoiceServerMessage;
      } catch {
        return;
      }
      if (msg.type === "hello") {
        this.attempt = 0;
        this.opts.onConnection("open");
      }
      this.opts.onMessage(msg);
    };
    ws.onclose = (ev) => {
      this.socket = null;
      if (ev.code === CLOSE_UNAUTHORIZED) {
        this.opts.onConnection("unauthorized");
        return; // reintentar con el mismo token no sirve
      }
      this.opts.onConnection("disconnected");
      if (!this.stopped) {
        const wait = backoffMs(this.attempt++);
        this.opts.schedule(() => this.open(), wait);
      }
    };
    ws.onerror = () => {
      // onclose se encarga de reintentar.
    };
  }

  send(msg: VoiceClientMessage): boolean {
    if (!this.socket || this.socket.readyState !== 1) return false;
    this.socket.send(JSON.stringify(msg));
    return true;
  }

  close(): void {
    this.stopped = true;
    this.socket?.close();
    this.socket = null;
  }
}
