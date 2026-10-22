import { useEffect, useState } from "react";
import type { AuditEntry } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Empty, ScreenHeader, formatDate } from "../../components/ui";

const ACTION_LABEL: Record<string, string> = {
  authorize: "Autorización",
  execute: "Ejecución",
  unknown_tool: "Herramienta inexistente",
  invalid_arguments: "Parámetros inválidos",
  llm_error: "Error del modelo",
  set_tool_mode: "Cambio de permisos",
};

export function ActivityScreen() {
  const [entries, setEntries] = useState<AuditEntry[]>([]);

  useEffect(() => {
    const load = () => void backend().listAudit(300).then(setEntries).catch(() => undefined);
    load();
    const t = setInterval(load, 4000);
    return () => clearInterval(t);
  }, []);

  return (
    <div className="mx-auto max-w-5xl">
      <ScreenHeader title="Historial de actividad" />
      <p className="mb-4 text-sm text-mist">Cada decisión de permisos y cada acción ejecutada, en orden.</p>
      {entries.length === 0 ? (
        <Empty>Sin actividad registrada.</Empty>
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-mist">
            <tr>
              <th className="py-2 font-normal">Hora</th>
              <th className="py-2 font-normal">Qué</th>
              <th className="py-2 font-normal">Herramienta</th>
              <th className="py-2 font-normal">Quién decidió</th>
              <th className="py-2 font-normal">Resultado</th>
            </tr>
          </thead>
          <tbody>
            {entries.map((e) => {
              const result = e.outcome ?? e.decision ?? "";
              const bad = result === "failed" || result === "deny";
              return (
                <tr key={e.id} className="border-t border-tide/60 align-top" title={e.detail ?? undefined}>
                  <td className="py-2 pr-3 whitespace-nowrap text-mist">{formatDate(e.ts)}</td>
                  <td className="py-2 pr-3 text-ice">{ACTION_LABEL[e.action] ?? e.action}</td>
                  <td className="py-2 pr-3 font-mono text-xs text-ice">{e.tool ?? "—"}</td>
                  <td className="py-2 pr-3 text-mist">{e.actor}</td>
                  <td className={`py-2 ${bad ? "text-coral" : "text-kelp"}`}>{result}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
