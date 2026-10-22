// Une voz y agente: transcripción → instrucción; respuesta final → voz.
import { useAssistant } from "../../stores/assistantStore";
import { useVoice } from "../../stores/voiceStore";
import { toSpeech } from "../../services/text";

let installed = false;

export function installVoiceBridge(): () => void {
  if (installed) return () => undefined;
  installed = true;

  useVoice.setState({
    onTranscript: (text) => {
      const assistant = useAssistant.getState();
      if (assistant.activeRun) {
        // Jarvis ya está trabajando: no se encola una segunda orden.
        useVoice.getState().agentDone();
        return;
      }
      useVoice.getState().thinking();
      void assistant.send(text, { spoken: true });
    },
  });

  const unsubscribe = useAssistant.subscribe((state) => {
    for (const m of state.messages) {
      if (m.role !== "assistant" || !m.spoken || m.spokenHandled || m.streaming) continue;
      useAssistant.getState().markSpoken(m.id);
      const voice = useVoice.getState();
      if (m.error || m.cancelled || !m.text.trim()) {
        voice.agentDone();
      } else {
        voice.speak(toSpeech(m.text));
      }
    }
  });

  return () => {
    installed = false;
    unsubscribe();
    useVoice.setState({ onTranscript: null });
  };
}
