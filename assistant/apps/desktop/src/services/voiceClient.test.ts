import { describe, expect, it } from "vitest";
import { backoffMs, CLOSE_UNAUTHORIZED, VoiceClient, type ConnectionState } from "./voiceClient";

class FakeSocket {
  static all: FakeSocket[] = [];
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: ((e: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {
    FakeSocket.all.push(this);
  }
  send(d: string) {
    this.sent.push(d);
  }
  close() {
    this.onclose?.({ code: 1000 });
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
}

function setup() {
  FakeSocket.all = [];
  const states: ConnectionState[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  const client = new VoiceClient({
    url: "ws://127.0.0.1:8765/ws",
    token: "secreto",
    onMessage: () => undefined,
    onConnection: (s) => states.push(s),
    createSocket: (u) => new FakeSocket(u) as unknown as WebSocket,
    schedule: (fn, ms) => timers.push({ fn, ms }),
  });
  return { client, states, timers };
}

describe("VoiceClient", () => {
  it("la espera crece exponencialmente con tope", () => {
    expect([0, 1, 2, 3].map(backoffMs)).toEqual([500, 1000, 2000, 4000]);
    expect(backoffMs(20)).toBe(15_000);
  });

  it("se autentica con el primer mensaje y queda abierto tras hello", () => {
    const { client, states } = setup();
    client.connect();
    const ws = FakeSocket.all[0];
    ws.open();
    expect(JSON.parse(ws.sent[0])).toEqual({ type: "auth", token: "secreto" });
    ws.onmessage?.({ data: JSON.stringify({ type: "hello", state: "IDLE", listening: false, wakeWord: null }) });
    expect(states.at(-1)).toBe("open");
    expect(client.send({ type: "ptt_start" })).toBe(true);
  });

  it("reconecta tras un cierre inesperado y no lo hace si el token es inválido", () => {
    const { client, states, timers } = setup();
    client.connect();
    FakeSocket.all[0].onclose?.({ code: 1006 });
    expect(states.at(-1)).toBe("disconnected");
    expect(timers[0].ms).toBe(500);
    timers[0].fn();
    expect(FakeSocket.all).toHaveLength(2);

    FakeSocket.all[1].onclose?.({ code: CLOSE_UNAUTHORIZED });
    expect(states.at(-1)).toBe("unauthorized");
    expect(timers).toHaveLength(1);
  });

  it("no reconecta después de close()", () => {
    const { client, timers } = setup();
    client.connect();
    client.close();
    expect(timers).toHaveLength(0);
  });
});
