import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { VoiceHealth } from "@jarvis/shared-types";
import { useVoice } from "../../stores/voiceStore";
import { createMockBackend, setBackend } from "../../services/backend";
import type { VoiceApi, VoiceConfig } from "../../services/voiceApi";
import { VoiceSettings } from "./VoiceSettings";

afterEach(() => {
  cleanup();
  useVoice.getState().disconnect();
  useVoice.setState({ api: null, connection: "disconnected", serverError: null });
});

const config: VoiceConfig = {
  input_device: null,
  output_device: null,
  volume: 0.9,
  listening_enabled: true,
  tts_provider: "server",
  tts_speed: 1,
  wake_word_enabled: false,
};

const health = (connected: boolean): VoiceHealth => ({
  status: "ok",
  state: "IDLE",
  stt: { ready: connected, provider: "server", detail: connected ? "servidor · modelo base listo" : "sin conexión con tu servidor" },
  tts: { ready: connected, provider: "server", detail: connected ? "servidor · voz es_MX-claude-high lista" : "sin conexión con tu servidor" },
  audio: { ready: true, provider: "sounddevice", detail: "ok" },
  wakeWord: { enabled: false, available: false, model: null, detail: "" },
  server: { connected, url: connected ? "https://jarvis.example.com" : null },
});

function fakeApi(connected = true) {
  const api = {
    config: vi.fn(async () => config),
    devices: vi.fn(async () => []),
    health: vi.fn(async () => health(connected)),
    updateConfig: vi.fn(async (patch: Partial<VoiceConfig>) => ({ ...config, ...patch })),
    check: vi.fn(async () => ({ ok: connected, stt: { ok: connected, detail: "stt" }, tts: { ok: connected, detail: "el servicio /tts no está disponible en el servidor (502)" } })),
    setServer: vi.fn(async (url: string) => ({ connected: true, url, check: null })),
  };
  useVoice.setState({ api: api as unknown as VoiceApi, connection: "open" });
  return api;
}

describe("Configuración → Voz", () => {
  it("muestra que transcripción y voz están en el servidor y sin opciones de clonación", async () => {
    fakeApi(true);
    render(<VoiceSettings />);
    await waitFor(() => expect(screen.getByText("servidor · modelo base listo")).toBeTruthy());
    expect(screen.getByText("https://jarvis.example.com")).toBeTruthy();
    expect(screen.getByText(/voz es_MX-claude-high lista/)).toBeTruthy();
    expect(screen.queryByText(/Voces guardadas|Nueva voz|OpenVoice|VibeVoice|Modelo de transcripción/)).toBeNull();
    const voice = screen.getByLabelText("Voz de Jarvis") as HTMLSelectElement;
    expect(voice.value).toBe("server");
    expect([...voice.options].map((o) => o.value)).toEqual(["server", "sapi"]);
  });

  it("«Probar» explica qué falla en el servidor", async () => {
    fakeApi(false);
    render(<VoiceSettings />);
    await waitFor(() => expect(screen.getByText(/empareja la PC en Infraestructura/)).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Probar" }));
    await waitFor(() => expect(screen.getByText(/no está disponible en el servidor/)).toBeTruthy());
  });
});

describe("Credencial del servidor para el servicio de voz", () => {
  it("se entrega al conectar y se renueva cuando el servidor la rechaza", async () => {
    const b = createMockBackend(0, { demoGraph: false });
    setBackend(b);
    const api = fakeApi(true);
    await useVoice.getState().pushServerCredential();
    expect(useVoice.getState().serverError).toMatch(/no está emparejado/);
    expect(api.setServer).not.toHaveBeenCalled();

    await b.cloudPair({ url: "https://jarvis.example.com", email: "yo@example.com", password: "x", deviceName: "PC" });
    useVoice.getState().handle({ type: "error", code: "server_auth", message: "credencial caducada" });
    await waitFor(() => expect(api.setServer).toHaveBeenCalledWith("https://jarvis.example.com", "token-de-vista-previa"));
    expect(useVoice.getState().serverError).toBeNull();
  });
});
