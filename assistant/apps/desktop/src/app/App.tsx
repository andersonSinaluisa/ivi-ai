import { useEffect } from "react";
import { Sidebar } from "../components/Sidebar";
import { Notice } from "../components/ui";
import { backend } from "../services/backend";
import { useStatus } from "../stores/statusStore";
import { useUi } from "../stores/uiStore";
import { useVoice } from "../stores/voiceStore";
import { DashboardScreen } from "../features/dashboard/DashboardScreen";
import { ChatScreen } from "../features/assistant/ChatScreen";
import { TasksScreen } from "../features/tasks/TasksScreen";
import { MemoryScreen } from "../features/memory/MemoryScreen";
import { SkillsScreen } from "../features/skills/SkillsScreen";
import { ActivityScreen } from "../features/activity/ActivityScreen";
import { SettingsScreen } from "../features/settings/SettingsScreen";
import { installVoiceBridge } from "../features/voice/voiceBridge";
import { useGraph } from "../stores/graphStore";

const SCREENS = {
  dashboard: DashboardScreen,
  chat: ChatScreen,
  tasks: TasksScreen,
  memory: MemoryScreen,
  skills: SkillsScreen,
  activity: ActivityScreen,
  settings: SettingsScreen,
} as const;

export function App() {
  const screen = useUi((s) => s.screen);
  const refresh = useStatus((s) => s.refresh);
  const statusError = useStatus((s) => s.error);
  const Screen = SCREENS[screen];

  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), 15_000);
    const uninstall = installVoiceBridge();
    // Eventos visuales reales del Memory Engine y del agente → grafo.
    let stopVisual: (() => void) | null = null;
    let disposed = false;
    void backend()
      .onVisualEvent((e) => useGraph.getState().apply(e))
      .then((stop) => {
        if (disposed) stop();
        else stopVisual = stop;
      });
    void useVoice.getState().connect();
    // Si el servicio de voz arranca después que la app, se reintenta obtener el token.
    const v = setInterval(() => {
      const s = useVoice.getState();
      if (!s.client) void s.connect();
    }, 10_000);
    return () => {
      clearInterval(t);
      clearInterval(v);
      uninstall();
      disposed = true;
      stopVisual?.();
      useVoice.getState().disconnect();
    };
  }, [refresh]);

  return (
    <div className="flex h-full">
      <Sidebar />
      <main className="flex min-w-0 flex-1 flex-col px-8 py-6">
        {backend().kind === "mock" && (
          <div className="mb-4">
            <Notice tone="warn">Vista previa en el navegador: los datos son simulados. Abre Jarvis con «npm run dev» para usar el backend real.</Notice>
          </div>
        )}
        {statusError && (
          <div className="mb-4">
            <Notice tone="error">{statusError}</Notice>
          </div>
        )}
        <div className={`min-h-0 flex-1 ${screen === "chat" ? "" : "overflow-y-auto"}`}>
          <Screen />
        </div>
      </main>
    </div>
  );
}
