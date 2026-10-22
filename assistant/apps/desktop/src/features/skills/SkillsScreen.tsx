// Herramientas disponibles y sus permisos. Las experiencias y habilidades aprendidas están en Memoria.
import { useCallback, useEffect, useState } from "react";
import type { ToolInfo, ToolPolicyMode } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Button, Notice, RiskTag, ScreenHeader } from "../../components/ui";

const MODE_LABEL: Record<ToolPolicyMode, string> = {
  default: "Según su riesgo",
  allow: "Permitir siempre",
  confirm: "Preguntar siempre",
  deny: "Bloquear",
};
const DECISION_LABEL = { allow: "Se ejecuta sin preguntar", confirm: "Pide tu autorización", deny: "Bloqueada" } as const;

export function SkillsScreen() {
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [agentDetail, setAgentDetail] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    void backend().listTools().then(setTools).catch((e: Error) => setError(e.message));
  }, []);
  useEffect(load, [load]);

  const reconnect = async () => {
    setBusy(true);
    const s = await backend().refreshWindowsAgent();
    setAgentDetail(s.detail);
    setBusy(false);
    load();
  };

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-10">
      <section>
        <ScreenHeader title="Herramientas y permisos">
          <Button onClick={() => void reconnect()} disabled={busy}>
            {busy ? "Conectando…" : "Reconectar windows-agent"}
          </Button>
        </ScreenHeader>
        {agentDetail && <Notice>{agentDetail}</Notice>}
        {error && <Notice tone="error">{error}</Notice>}
        <p className="mb-3 text-sm text-mist">
          Las acciones destructivas siempre piden autorización, aunque elijas «Permitir siempre».
        </p>
        <ul className="divide-y divide-tide/60">
          {tools.map((t) => (
            <li key={t.id} className="grid grid-cols-[minmax(0,1fr)_200px] gap-4 py-3">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-sm text-ice">{t.name}</span>
                  <RiskTag risk={t.risk} />
                  <span className="text-xs text-mist">{t.source === "local" ? "Local" : "windows-agent"}</span>
                </div>
                <p className="text-sm text-mist">{t.description}</p>
                {t.permissions.length > 0 && <p className="text-xs text-mist/80">Permisos: {t.permissions.join(", ")}</p>}
              </div>
              <div className="flex flex-col gap-1">
                <select
                  aria-label={`Política de ${t.name}`}
                  value={t.policyMode}
                  onChange={(e) => void backend().setToolMode(t.name, e.target.value as ToolPolicyMode).then(load)}
                  className="rounded-lg border border-tide bg-deep px-2 py-1.5 text-sm"
                >
                  {(Object.keys(MODE_LABEL) as ToolPolicyMode[]).map((m) => (
                    <option key={m} value={m}>
                      {MODE_LABEL[m]}
                    </option>
                  ))}
                </select>
                <span className="text-xs text-mist">{DECISION_LABEL[t.effectiveDecision]}</span>
              </div>
            </li>
          ))}
        </ul>
      </section>

    </div>
  );
}
