// Controles de voz: pulsar para hablar (botón o barra espaciadora fuera de campos de texto),
// escucha activa y detener respuesta hablada.
import { useEffect, useRef } from "react";
import type { VoiceState } from "@jarvis/shared-types";
import { useVoice } from "../../stores/voiceStore";
import { Button } from "../../components/ui";

export const VOICE_LABEL: Record<VoiceState, string> = {
  IDLE: "En espera",
  LISTENING_WAKE_WORD: "Escuchando la palabra de activación",
  WAKE_DETECTED: "Te escuché",
  RECORDING_COMMAND: "Grabando tu instrucción",
  TRANSCRIBING: "Transcribiendo",
  THINKING: "Pensando",
  SPEAKING: "Hablando",
  ERROR: "Error de voz",
};

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}

export function VoiceDock({ compact = false }: { compact?: boolean }) {
  const v = useVoice();
  const holding = useRef(false);
  const connected = v.connection === "open";

  useEffect(() => {
    if (!connected) return;
    const down = (e: KeyboardEvent) => {
      if (e.code !== "Space" || e.repeat || isTyping(e.target)) return;
      e.preventDefault();
      holding.current = true;
      useVoice.getState().pttStart();
    };
    const up = (e: KeyboardEvent) => {
      if (e.code !== "Space" || !holding.current) return;
      holding.current = false;
      useVoice.getState().pttStop();
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, [connected]);

  if (!connected) {
    return (
      <div className="text-sm text-mist">
        {v.connection === "connecting" ? "Conectando con el servicio de voz…" : (v.unavailableReason ?? "Servicio de voz sin conexión.")}
      </div>
    );
  }

  const recording = v.state === "RECORDING_COMMAND";
  return (
    <div className={`flex ${compact ? "items-center gap-2" : "flex-col gap-3"}`}>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onPointerDown={() => v.pttStart()}
          onPointerUp={() => v.pttStop()}
          onPointerLeave={() => recording && v.pttStop()}
          aria-pressed={recording}
          className={`rounded-full px-4 py-2 text-sm font-medium transition-colors ${
            recording ? "bg-cyan text-abyss" : "border border-current text-ice hover:bg-deep"
          }`}
        >
          {recording ? "Suelta para enviar" : "Mantén para hablar"}
        </button>
        {v.state === "SPEAKING" && (
          <Button onClick={() => v.stopSpeaking()}>Callar</Button>
        )}
        {(v.state === "TRANSCRIBING" || v.state === "THINKING" || recording) && (
          <Button onClick={() => v.cancel()}>Cancelar</Button>
        )}
      </div>
      {!compact && (
        <div className="flex flex-col gap-1 text-sm text-mist">
          <span aria-live="polite">{VOICE_LABEL[v.state]}</span>
          <span>También puedes mantener la barra espaciadora.</span>
          {v.wakeWord && (
            <span>
              Palabra de activación: {v.wakeWord.available && v.wakeWord.enabled ? `activa (${v.wakeWord.model})` : v.wakeWord.detail}
            </span>
          )}
          {v.wakeWord?.available && (
            <label className="flex items-center gap-2">
              <input type="checkbox" checked={v.listening} onChange={(e) => v.setListening(e.target.checked)} />
              Escuchar la palabra de activación
            </label>
          )}
          {v.lastError && <span className="text-coral">{v.lastError}</span>}
        </div>
      )}
    </div>
  );
}
