import { useCallback, useEffect, useState } from "react";
import type { MemoryStats } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Button, Notice, ScreenHeader } from "../../components/ui";
import { MemoryExplorer } from "./MemoryExplorer";
import { ExperiencesPanel } from "./ExperiencesPanel";
import { TransferPanel } from "./TransferPanel";
import { useGraph } from "../../stores/graphStore";

type Tab = "explore" | "learned" | "transfer";

const TABS: { id: Tab; label: string }[] = [
  { id: "explore", label: "Explorar" },
  { id: "learned", label: "Experiencias y habilidades" },
  { id: "transfer", label: "Exportar e importar" },
];

export function MemoryScreen() {
  const [tab, setTab] = useState<Tab>("explore");
  const [stats, setStats] = useState<MemoryStats | null>(null);
  const [message, setMessage] = useState<{ tone: "info" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const version = useGraph((s) => s.version);

  const loadStats = useCallback(() => {
    void backend().memoryStats().then(setStats).catch(() => setStats(null));
  }, []);
  useEffect(loadStats, [loadStats, version]);

  const run = async (label: string, fn: () => Promise<string>) => {
    setBusy(true);
    setMessage(null);
    try {
      setMessage({ tone: "info", text: await fn() });
    } catch (e) {
      setMessage({ tone: "error", text: `${label}: ${e instanceof Error ? e.message : String(e)}` });
    } finally {
      setBusy(false);
      loadStats();
    }
  };

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-4">
      <ScreenHeader title="Memoria">
        <Button disabled={busy} onClick={() => void run("Consolidar", async () => {
          const r = await backend().consolidateMemory();
          return `Consolidado: ${r.removedExpired} vencidos eliminados, ${r.mergedDuplicates} duplicados fusionados, ${r.prunedRelations} relaciones débiles podadas.`;
        })}>
          Consolidar
        </Button>
      </ScreenHeader>

      {stats && (
        <div className="flex flex-wrap items-baseline gap-x-6 gap-y-1 text-sm text-mist">
          <span><strong className="text-ice">{stats.total}</strong> recuerdos</span>
          <span><strong className="text-ice">{stats.relations}</strong> relaciones</span>
          <span><strong className="text-ice">{stats.skills}</strong> habilidades</span>
          <span className={stats.embedding.semantic ? "" : "text-amber"}>
            Búsqueda {stats.embedding.semantic ? "semántica" : "léxica"}: {stats.embedding.detail}
            {stats.embedding.pending > 0 && ` · ${stats.embedding.pending} pendientes de vectorizar`}
          </span>
          <button type="button" className="text-cyan hover:underline disabled:opacity-40" disabled={busy} onClick={() => void run("Embeddings", () => backend().refreshEmbeddings())}>
            Reintentar modelo
          </button>
        </div>
      )}
      {message && <Notice tone={message.tone}>{message.text}</Notice>}

      <div role="tablist" aria-label="Secciones de memoria" className="flex gap-1 border-b border-tide/70">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            type="button"
            aria-selected={tab === t.id}
            onClick={() => setTab(t.id)}
            className={`-mb-px border-b-2 px-3 py-2 text-sm ${tab === t.id ? "border-cyan text-ice" : "border-transparent text-mist hover:text-ice"}`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === "explore" && <MemoryExplorer onChanged={loadStats} />}
      {tab === "learned" && <ExperiencesPanel />}
      {tab === "transfer" && <TransferPanel exportsDir={stats?.exportsDir ?? ""} onChanged={loadStats} />}
    </div>
  );
}
