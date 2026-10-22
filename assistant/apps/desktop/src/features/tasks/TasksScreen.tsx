import { useEffect, useState } from "react";
import type { RunSummary } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { useAssistant } from "../../stores/assistantStore";
import { Button, Empty, ScreenHeader, formatDate } from "../../components/ui";

const STATUS: Record<RunSummary["status"], { text: string; cls: string }> = {
  running: { text: "En curso", cls: "text-cyan" },
  completed: { text: "Completada", cls: "text-kelp" },
  failed: { text: "Falló", cls: "text-coral" },
  cancelled: { text: "Cancelada", cls: "text-mist" },
};

export function TasksScreen() {
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const active = useAssistant((s) => s.activeRun);
  const cancel = useAssistant((s) => s.cancel);

  useEffect(() => {
    const load = () => void backend().listRuns(100).then(setRuns).catch(() => undefined);
    load();
    const t = setInterval(load, 3000);
    return () => clearInterval(t);
  }, []);

  return (
    <div className="mx-auto max-w-4xl">
      <ScreenHeader title="Tareas">
        {active && (
          <Button tone="danger" onClick={() => void cancel()}>
            Detener la tarea en curso
          </Button>
        )}
      </ScreenHeader>
      {runs.length === 0 ? (
        <Empty>Todavía no hay tareas. Cada instrucción que le das a Jarvis aparece aquí.</Empty>
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-mist">
            <tr>
              <th className="py-2 font-normal">Instrucción</th>
              <th className="py-2 font-normal">Estado</th>
              <th className="py-2 font-normal">Herramientas</th>
              <th className="py-2 font-normal">Inicio</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((r) => (
              <tr key={r.id} className="border-t border-tide/60 align-top">
                <td className="max-w-md py-2 pr-4">
                  <p className="text-ice">{r.goal}</p>
                  {r.error && <p className="text-xs text-coral">{r.error}</p>}
                </td>
                <td className={`py-2 ${STATUS[r.status].cls}`}>{STATUS[r.status].text}</td>
                <td className="py-2 text-mist">{r.toolCalls}</td>
                <td className="py-2 text-mist">{formatDate(r.startedAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
