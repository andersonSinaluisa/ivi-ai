import type { Screen } from "../stores/uiStore";
import { useUi } from "../stores/uiStore";
import { useAssistant } from "../stores/assistantStore";
import { useVoice } from "../stores/voiceStore";

const ITEMS: { id: Screen; label: string; icon: string }[] = [
  { id: "dashboard", label: "Inicio", icon: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm0 5a4 4 0 1 1 0 8 4 4 0 0 1 0-8Z" },
  { id: "chat", label: "Conversación", icon: "M4 5h16v10H8l-4 4V5Z" },
  { id: "tasks", label: "Tareas", icon: "M5 6h14M5 12h14M5 18h9" },
  { id: "memory", label: "Memoria", icon: "M6 4h12v16l-6-4-6 4V4Z" },
  { id: "skills", label: "Herramientas", icon: "M13 3 5 13h6l-1 8 8-10h-6l1-8Z" },
  { id: "activity", label: "Actividad", icon: "M3 12h4l3-7 4 14 3-7h4" },
  { id: "settings", label: "Configuración", icon: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6Zm8 3-2-1 .5-2.2-1.6-1.6L14.7 7 13.7 5h-3.4l-1 2-2.2-.8-1.6 1.6L6 10l-2 2 2 1-.5 2.2 1.6 1.6 2.2-.8 1 2h3.4l1-2 2.2.8 1.6-1.6L18 13l2-1Z" },
];

export function Sidebar() {
  const screen = useUi((s) => s.screen);
  const go = useUi((s) => s.go);
  const busy = useAssistant((s) => s.activeRun !== null);
  const voice = useVoice((s) => s.connection);

  return (
    <nav aria-label="Secciones" className="flex w-52 shrink-0 flex-col border-r border-tide/70 bg-abyss/60 px-3 py-5">
      <div className="mb-8 flex items-center gap-2 px-2">
        <span aria-hidden className={`h-3 w-3 rounded-full ${busy ? "bg-cyan shadow-[0_0_12px_#22d3ee]" : "bg-current"}`} />
        <span className="font-display text-lg font-semibold tracking-tight">Jarvis</span>
      </div>
      <ul className="flex flex-col gap-1">
        {ITEMS.map((it) => {
          const active = it.id === screen;
          return (
            <li key={it.id}>
              <button
                type="button"
                onClick={() => go(it.id)}
                aria-current={active ? "page" : undefined}
                className={`flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left text-sm transition-colors ${
                  active ? "bg-deep text-ice" : "text-mist hover:bg-deep/60 hover:text-ice"
                }`}
              >
                <svg viewBox="0 0 24 24" className={`h-4 w-4 ${active ? "text-cyan" : ""}`} fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" aria-hidden>
                  <path d={it.icon} />
                </svg>
                {it.label}
              </button>
            </li>
          );
        })}
      </ul>
      <p className="mt-auto px-2 text-xs text-mist">
        Voz: {voice === "open" ? "conectada" : voice === "connecting" ? "conectando…" : "sin conexión"}
      </p>
    </nav>
  );
}
