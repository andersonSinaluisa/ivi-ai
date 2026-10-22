import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useVoice } from "../../stores/voiceStore";
import type { VoiceApi, VoiceConfig } from "../../services/voiceApi";
import { TranscriptionSettings } from "./VoiceSettings";

afterEach(() => {
  cleanup();
  useVoice.setState({ api: null });
});

const base: VoiceConfig = {
  input_device: null,
  output_device: null,
  volume: 0.9,
  listening_enabled: true,
  tts_provider: "sapi",
  tts_speed: 1,
  whisper_model: "base",
  wake_word_enabled: false,
  stt_provider: "faster-whisper",
  stt_remote_url: "",
  stt_remote_token_set: false,
};

function fakeApi(checkOk = true) {
  let config = { ...base };
  const api = {
    updateConfig: vi.fn(async (patch: Partial<VoiceConfig>) => (config = { ...config, ...patch })),
    setRemoteSttToken: vi.fn(async (token: string) => (config = { ...config, stt_remote_token_set: token !== "" })),
    checkStt: vi.fn(async () => ({ ok: checkOk, provider: "remote", detail: checkOk ? "servidor listo · modelo base listo" : "el servidor rechazó el token (401)" })),
  };
  useVoice.setState({ api: api as unknown as VoiceApi });
  return api;
}

describe("Transcripción en el servidor", () => {
  it("guarda la URL y el token, cambia al servidor y lo comprueba", async () => {
    const api = fakeApi();
    let current = base;
    const { rerender } = render(<TranscriptionSettings config={current} onChange={(c) => (current = c)} />);
    expect((screen.getByRole("radio", { name: "En el servidor" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByPlaceholderText(/tu-dominio\/stt/), { target: { value: "https://jarvis.example.com/stt" } });
    fireEvent.change(screen.getByLabelText("Token"), { target: { value: "secreto" } });
    fireEvent.click(screen.getByRole("button", { name: "Guardar y usar el servidor" }));
    await waitFor(() => expect(screen.getByText(/Servidor listo/)).toBeTruthy());
    expect(api.updateConfig).toHaveBeenCalledWith({ stt_provider: "remote", stt_remote_url: "https://jarvis.example.com/stt" });
    expect(api.setRemoteSttToken).toHaveBeenCalledWith("secreto");
    expect(current.stt_provider).toBe("remote");
    rerender(<TranscriptionSettings config={current} onChange={(c) => (current = c)} />);
    expect(screen.getByRole("radio", { name: "En el servidor" }).getAttribute("aria-checked")).toBe("true");
    expect((screen.getByLabelText("Token") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Token") as HTMLInputElement).placeholder).toContain("guardado");
  });

  it("muestra por qué falla el servidor", async () => {
    fakeApi(false);
    render(<TranscriptionSettings config={{ ...base, stt_remote_url: "https://jarvis.example.com/stt" }} onChange={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Probar" }));
    await waitFor(() => expect(screen.getByText(/rechazó el token/)).toBeTruthy());
  });
});
