// Explorar recuerdos: búsqueda por texto o semántica, filtros, carga progresiva y detalle.
import { useCallback, useEffect, useState } from "react";
import type { MemoryFilter, MemoryKind, MemoryRecord, Verification } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Button, Empty, Notice } from "../../components/ui";
import { KIND_COLOR, KIND_LABEL } from "../neural/layout";
import { useUi } from "../../stores/uiStore";
import { MemoryDetailPanel } from "./MemoryDetailPanel";

const PAGE = 40;
export const VERIFICATION_LABEL: Record<Verification, string> = { verified: "Verificado", unverified: "Sin verificar", disputed: "En duda" };
const input = "rounded-lg border border-tide bg-deep/70 px-3 py-1.5 text-sm focus:border-cyan focus:outline-none";

export function MemoryExplorer({ onChanged }: { onChanged(): void }) {
  const focus = useUi((s) => s.memoryFocus);
  const openMemory = useUi((s) => s.openMemory);
  const [filter, setFilter] = useState<MemoryFilter>({});
  const [query, setQuery] = useState("");
  const [semantic, setSemantic] = useState(false);
  const [items, setItems] = useState<MemoryRecord[]>([]);
  const [scores, setScores] = useState<Record<string, number>>({});
  const [total, setTotal] = useState(0);
  const [selected, setSelected] = useState<string | null>(focus);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [draftKind, setDraftKind] = useState<MemoryKind>("semantic");

  useEffect(() => {
    if (focus) {
      setSelected(focus);
      openMemory(null);
    }
  }, [focus, openMemory]);

  const load = useCallback(
    async (offset: number) => {
      setError(null);
      try {
        if (semantic && query.trim()) {
          const r = await backend().searchMemories(query);
          setItems(r.items.map((i) => i.memory));
          setScores(Object.fromEntries(r.items.map((i) => [i.memory.id, i.score])));
          setTotal(r.items.length);
          return;
        }
        const page = await backend().listMemories({ ...filter, query: query || undefined }, offset, PAGE);
        setScores({});
        setItems((prev) => (offset === 0 ? page.items : [...prev, ...page.items]));
        setTotal(page.total);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [filter, query, semantic],
  );

  useEffect(() => {
    const t = setTimeout(() => void load(0), 250);
    return () => clearTimeout(t);
  }, [load]);

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!draft.trim()) return;
    try {
      const out = await backend().addMemory({ content: draft, kind: draftKind });
      setDraft("");
      setSelected(out.memory.id);
      if (!out.created) setError("Ese recuerdo ya existía: lo abrí.");
      await load(0);
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const kinds: (MemoryKind | undefined)[] = [undefined, "semantic", "entity", "procedural", "experience"];

  return (
    <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] gap-6">
      <section className="flex min-w-0 flex-col gap-3">
        <form onSubmit={add} className="flex gap-2">
          <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Algo que Jarvis deba recordar" aria-label="Nuevo recuerdo" className={`${input} flex-1`} />
          <select aria-label="Tipo del nuevo recuerdo" value={draftKind} onChange={(e) => setDraftKind(e.target.value as MemoryKind)} className={input}>
            {(["semantic", "entity", "procedural"] as MemoryKind[]).map((k) => (
              <option key={k} value={k}>{KIND_LABEL[k]}</option>
            ))}
          </select>
          <Button tone="primary" type="submit" disabled={!draft.trim()}>
            Recordar
          </Button>
        </form>

        <div className="flex flex-wrap items-center gap-2">
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Buscar" aria-label="Buscar recuerdos" className={`${input} min-w-40 flex-1`} />
          <label className="flex items-center gap-1 text-sm text-mist">
            <input type="checkbox" checked={semantic} onChange={(e) => setSemantic(e.target.checked)} />
            Por significado
          </label>
          <select
            aria-label="Verificación"
            value={filter.verification ?? ""}
            onChange={(e) => setFilter({ ...filter, verification: (e.target.value || undefined) as Verification | undefined })}
            className={input}
          >
            <option value="">Cualquier estado</option>
            {(Object.keys(VERIFICATION_LABEL) as Verification[]).map((v) => (
              <option key={v} value={v}>{VERIFICATION_LABEL[v]}</option>
            ))}
          </select>
        </div>
        <div className="flex flex-wrap gap-1" role="group" aria-label="Tipo">
          {kinds.map((k) => {
            const active = filter.kind === k;
            return (
              <button
                key={k ?? "all"}
                type="button"
                aria-pressed={active}
                onClick={() => setFilter({ ...filter, kind: k })}
                className={`rounded-full border px-3 py-1 text-xs ${active ? "border-cyan bg-cyan/10 text-cyan" : "border-tide text-mist hover:text-ice"}`}
              >
                {k ? KIND_LABEL[k] : "Todos"}
              </button>
            );
          })}
        </div>

        {error && <Notice tone="error">{error}</Notice>}
        {items.length === 0 ? (
          <Empty>{query ? "Nada coincide con la búsqueda." : "Aún no hay recuerdos con este filtro."}</Empty>
        ) : (
          <ul className="divide-y divide-tide/60">
            {items.map((m) => (
              <li key={m.id}>
                <button
                  type="button"
                  onClick={() => setSelected(m.id)}
                  aria-current={selected === m.id ? "true" : undefined}
                  className={`flex w-full items-start gap-3 px-2 py-2.5 text-left ${selected === m.id ? "bg-deep" : "hover:bg-deep/50"}`}
                >
                  <span aria-hidden className="mt-1.5 inline-block h-2 w-2 shrink-0 rounded-full" style={{ background: KIND_COLOR[m.kind] }} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm text-ice">{m.summary || m.content}</span>
                    <span className="block text-xs text-mist">
                      {KIND_LABEL[m.kind]} · {VERIFICATION_LABEL[m.verification]} · {m.source}
                      {scores[m.id] !== undefined && ` · relevancia ${scores[m.id].toFixed(2)}`}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {!semantic && items.length < total && (
          <Button onClick={() => void load(items.length)}>Cargar más ({total - items.length} restantes)</Button>
        )}
      </section>

      <section className="min-w-0">
        {selected ? (
          <MemoryDetailPanel
            id={selected}
            onSelect={setSelected}
            onChanged={() => {
              void load(0);
              onChanged();
            }}
            onDeleted={() => {
              setSelected(null);
              void load(0);
              onChanged();
            }}
          />
        ) : (
          <Empty>Elige un recuerdo para ver su fuente, sus relaciones y editarlo.</Empty>
        )}
      </section>
    </div>
  );
}
