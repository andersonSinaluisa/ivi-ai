// Detalle de un recuerdo: editar, verificar, ver fuente y usos, y gestionar relaciones.
import { useCallback, useEffect, useMemo, useState } from "react";
import type { GraphView, MemoryDetail, MemoryRecord, Provenance, RelationKind, Verification } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Button, Notice, formatDate } from "../../components/ui";
import { KIND_COLOR, KIND_LABEL, fitAroundCore, layoutGraph } from "../neural/layout";
import { NeuralCore2D } from "../neural/NeuralCore2D";
import { VERIFICATION_LABEL } from "./MemoryExplorer";

export const RELATION_LABEL: Record<RelationKind, string> = {
  BELONGS_TO: "pertenece a",
  DEPENDS_ON: "depende de",
  CAUSED_BY: "causado por",
  SOLVED_BY: "resuelto con",
  RELATED_TO: "relacionado con",
  LEARNED_FROM: "aprendido de",
  USED_IN: "se usa en",
  DUPLICATE_OF: "duplicado de",
};
const PROVENANCE_LABEL: Record<Provenance, string> = { user: "tú", agent: "Jarvis", inferred: "inferida", import: "importada", consolidation: "consolidación" };
const EVENT_LABEL: Record<string, string> = {
  MEMORY_CREATED: "Creado",
  MEMORY_UPDATED: "Editado",
  MEMORY_RETRIEVED: "Usado en una respuesta",
  MEMORY_CONNECTED: "Relacionado",
  MEMORY_IMPORTED: "Importado",
};
const input = "w-full rounded-lg border border-tide bg-deep/70 px-3 py-1.5 text-sm focus:border-cyan focus:outline-none";

interface Props {
  id: string;
  onSelect(id: string): void;
  onChanged(): void;
  onDeleted(): void;
}

export function MemoryDetailPanel({ id, onSelect, onChanged, onDeleted }: Props) {
  const [detail, setDetail] = useState<MemoryDetail | null>(null);
  const [graph, setGraph] = useState<GraphView | null>(null);
  const [edit, setEdit] = useState<{ content: string; summary: string; tags: string; confidence: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [relTarget, setRelTarget] = useState("");
  const [relCandidates, setRelCandidates] = useState<MemoryRecord[]>([]);
  const [relKind, setRelKind] = useState<RelationKind>("RELATED_TO");

  const load = useCallback(async () => {
    setError(null);
    try {
      const [d, g] = await Promise.all([backend().getMemory(id), backend().memoryGraph({ focus: id, depth: 2, limit: 80 })]);
      setDetail(d);
      setGraph(g);
      setEdit({ content: d.memory.content, summary: d.memory.summary, tags: d.memory.tags.join(", "), confidence: d.memory.confidence });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [id]);

  useEffect(() => {
    setConfirmDelete(false);
    void load();
  }, [load]);

  useEffect(() => {
    if (relTarget.trim().length < 2) {
      setRelCandidates([]);
      return;
    }
    const t = setTimeout(() => {
      void backend().listMemories({ query: relTarget }, 0, 6).then((p) => setRelCandidates(p.items.filter((m) => m.id !== id)));
    }, 250);
    return () => clearTimeout(t);
  }, [relTarget, id]);

  const positions = useMemo(() => (graph ? fitAroundCore(layoutGraph(graph.nodes, graph.edges, undefined, 40), 3.5, 0) : new Map()), [graph]);

  const act = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
      await load();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  if (!detail || !edit) return error ? <Notice tone="error">{error}</Notice> : <p className="text-sm text-mist">Cargando…</p>;
  const m = detail.memory;
  const dirty = edit.content !== m.content || edit.summary !== m.summary || edit.tags !== m.tags.join(", ") || edit.confidence !== m.confidence;

  return (
    <div className="pane flex flex-col gap-4 p-5">
      <div className="flex items-center gap-2 text-xs text-mist">
        <span aria-hidden className="inline-block h-2 w-2 rounded-full" style={{ background: KIND_COLOR[m.kind] }} />
        {KIND_LABEL[m.kind]} · creado {formatDate(m.createdAt)} · editado {formatDate(m.updatedAt)}
      </div>

      {graph && graph.nodes.length > 1 && (
        <NeuralCore2D
          nodes={graph.nodes}
          edges={graph.edges}
          positions={positions}
          highlights={{}}
          births={{}}
          selectedId={m.id}
          core={{ level: 0, listening: false, speaking: false, busy: false, awaitingApproval: false, flashAt: null }}
          lowPower
          onSelect={(other) => other && other !== m.id && onSelect(other)}
          height={170}
          label={`Relaciones de este recuerdo: ${graph.nodes.length - 1} conectados`}
        />
      )}

      <label className="flex flex-col gap-1 text-sm">
        <span className="text-mist">Contenido</span>
        <textarea rows={3} value={edit.content} onChange={(e) => setEdit({ ...edit, content: e.target.value })} className={input} />
      </label>
      <div className="grid grid-cols-2 gap-3">
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-mist">Resumen</span>
          <input value={edit.summary} onChange={(e) => setEdit({ ...edit, summary: e.target.value })} className={input} />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-mist">Etiquetas (separadas por comas)</span>
          <input value={edit.tags} onChange={(e) => setEdit({ ...edit, tags: e.target.value })} className={input} />
        </label>
      </div>
      <label className="flex items-center gap-3 text-sm">
        <span className="w-28 text-mist">Confianza {Math.round(edit.confidence * 100)} %</span>
        <input type="range" min={0} max={1} step={0.05} value={edit.confidence} onChange={(e) => setEdit({ ...edit, confidence: Number(e.target.value) })} className="flex-1 accent-cyan" />
      </label>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          tone="primary"
          disabled={!dirty}
          onClick={() =>
            void act(() =>
              backend().updateMemory(m.id, {
                content: edit.content,
                summary: edit.summary,
                tags: edit.tags.split(",").map((t) => t.trim()).filter(Boolean),
                confidence: edit.confidence,
              }),
            )
          }
        >
          Guardar cambios
        </Button>
        <select
          aria-label="Estado de verificación"
          value={m.verification}
          onChange={(e) => void act(() => backend().setMemoryVerification(m.id, e.target.value as Verification))}
          className="rounded-lg border border-tide bg-deep px-2 py-1.5 text-sm"
        >
          {(Object.keys(VERIFICATION_LABEL) as Verification[]).map((v) => (
            <option key={v} value={v}>{VERIFICATION_LABEL[v]}</option>
          ))}
        </select>
        {confirmDelete ? (
          <>
            <span className="text-sm text-coral">¿Olvidar este recuerdo y sus relaciones?</span>
            <Button tone="danger" onClick={() => void backend().deleteMemory(m.id).then(onDeleted).catch((e: Error) => setError(e.message))}>
              Sí, olvidar
            </Button>
            <Button onClick={() => setConfirmDelete(false)}>Cancelar</Button>
          </>
        ) : (
          <Button tone="danger" className="ml-auto" onClick={() => setConfirmDelete(true)}>
            Olvidar
          </Button>
        )}
      </div>
      {error && <Notice tone="error">{error}</Notice>}

      <section>
        <h3 className="mb-1 text-sm font-semibold">Fuente</h3>
        <p className="text-sm text-mist">
          {detail.source ? `${detail.source.label} (${detail.source.kind})${detail.source.reference ? ` · ref. ${detail.source.reference.slice(0, 12)}` : ""}` : m.source}
        </p>
      </section>

      <section>
        <h3 className="mb-2 text-sm font-semibold">Relaciones</h3>
        {detail.relations.length === 0 && <p className="text-sm text-mist">Sin relaciones.</p>}
        <ul className="flex flex-col gap-1">
          {detail.relations.map(({ relation: r, other, outgoing }) => (
            <li key={r.id} className="flex flex-wrap items-center gap-2 text-sm">
              <span className="text-mist">{outgoing ? RELATION_LABEL[r.kind] : `← ${RELATION_LABEL[r.kind]}`}</span>
              <button type="button" className="max-w-xs truncate text-ice hover:text-cyan" onClick={() => onSelect(other.id)}>
                {other.label}
              </button>
              <span className={`text-xs ${r.verified ? "text-kelp" : "text-mist"}`}>
                {r.verified ? "verificada" : `${PROVENANCE_LABEL[r.provenance]} · ${Math.round(r.confidence * 100)} %`}
              </span>
              {!r.verified && (
                <button type="button" className="text-xs text-cyan hover:underline" onClick={() => void act(() => backend().verifyRelation(r.id, true))}>
                  Confirmar
                </button>
              )}
              <button type="button" className="text-xs text-mist hover:text-coral" onClick={() => void act(() => backend().deleteRelation(r.id))}>
                Quitar
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-3 flex flex-col gap-2">
          <div className="flex gap-2">
            <select aria-label="Tipo de relación" value={relKind} onChange={(e) => setRelKind(e.target.value as RelationKind)} className="rounded-lg border border-tide bg-deep px-2 py-1.5 text-sm">
              {(Object.keys(RELATION_LABEL) as RelationKind[]).map((k) => (
                <option key={k} value={k}>{RELATION_LABEL[k]}</option>
              ))}
            </select>
            <input value={relTarget} onChange={(e) => setRelTarget(e.target.value)} placeholder="Buscar el recuerdo a relacionar" aria-label="Recuerdo a relacionar" className={input} />
          </div>
          {relCandidates.length > 0 && (
            <ul className="flex flex-col">
              {relCandidates.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    className="w-full truncate rounded-md px-2 py-1 text-left text-sm text-ice hover:bg-deep"
                    onClick={() =>
                      void act(async () => {
                        await backend().addRelation(m.id, c.id, relKind);
                        setRelTarget("");
                      })
                    }
                  >
                    {RELATION_LABEL[relKind]} → {c.summary || c.content}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>

      {detail.events.length > 0 && (
        <section>
          <h3 className="mb-1 text-sm font-semibold">Historial</h3>
          <ul className="text-xs text-mist">
            {detail.events.slice(0, 8).map((e) => (
              <li key={e.id}>
                {formatDate(e.ts)} · {EVENT_LABEL[e.kind] ?? e.kind}
                {e.runId ? ` (tarea ${e.runId.slice(0, 8)})` : ""}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
