// Exportar e importar la memoria (.jarvismem cifrado) y restaurar copias de seguridad.
import { useEffect, useState } from "react";
import type { BackupInfo, ConflictStrategy, ExportReport, ImportPreview, ImportReport, MemoryKind } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Button, Notice, formatDate } from "../../components/ui";
import { KIND_LABEL } from "../neural/layout";

const input = "w-full rounded-lg border border-tide bg-deep/70 px-3 py-1.5 text-sm focus:border-cyan focus:outline-none";
const KINDS: MemoryKind[] = ["semantic", "entity", "procedural", "experience", "episodic"];
const CONFLICT_LABEL: Record<ConflictStrategy, string> = {
  keep_local: "Conservar el mío",
  keep_imported: "Usar el importado",
  keep_both: "Conservar ambos",
};

function kb(bytes: number): string {
  return bytes > 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(bytes / 1024)} KB`;
}

function ExportForm({ onDone }: { onDone(): void }) {
  const [path, setPath] = useState("");
  const [pass, setPass] = useState("");
  const [pass2, setPass2] = useState("");
  const [kinds, setKinds] = useState<MemoryKind[]>(KINDS);
  const [skills, setSkills] = useState(true);
  const [experiences, setExperiences] = useState(true);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<ExportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void backend().defaultExportPath().then(setPath).catch(() => undefined);
  }, []);

  const valid = pass.length >= 8 && pass === pass2 && kinds.length > 0;
  const run = async () => {
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      setReport(await backend().exportMemory(path || null, pass, { kinds, includeSkills: skills, includeExperiences: experiences }));
      setPass("");
      setPass2("");
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="pane flex flex-col gap-3 p-5">
      <h2 className="text-sm font-semibold">Exportar</h2>
      <p className="text-sm text-mist">Crea un archivo cifrado con tus recuerdos, relaciones y habilidades. No incluye conversaciones, contraseñas ni ajustes.</p>
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-mist">Archivo</span>
        <input value={path} onChange={(e) => setPath(e.target.value)} className={input} aria-label="Ruta del archivo de exportación" />
      </label>
      <div className="grid grid-cols-2 gap-3">
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-mist">Contraseña (mín. 8)</span>
          <input type="password" autoComplete="new-password" value={pass} onChange={(e) => setPass(e.target.value)} className={input} />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-mist">Repítela</span>
          <input type="password" autoComplete="new-password" value={pass2} onChange={(e) => setPass2(e.target.value)} className={input} />
        </label>
      </div>
      {pass2 && pass !== pass2 && <p className="text-xs text-coral">Las contraseñas no coinciden.</p>}
      <fieldset className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
        <legend className="mb-1 text-mist">Qué incluir</legend>
        {KINDS.map((k) => (
          <label key={k} className="flex items-center gap-1">
            <input type="checkbox" checked={kinds.includes(k)} onChange={(e) => setKinds(e.target.checked ? [...kinds, k] : kinds.filter((x) => x !== k))} />
            {KIND_LABEL[k]}
          </label>
        ))}
        <label className="flex items-center gap-1"><input type="checkbox" checked={experiences} onChange={(e) => setExperiences(e.target.checked)} />Registro de tareas</label>
        <label className="flex items-center gap-1"><input type="checkbox" checked={skills} onChange={(e) => setSkills(e.target.checked)} />Habilidades</label>
      </fieldset>
      <p className="text-xs text-mist">Guarda la contraseña en un lugar seguro: sin ella el archivo no se puede abrir.</p>
      <div>
        <Button tone="primary" disabled={!valid || busy} onClick={() => void run()}>
          {busy ? "Exportando…" : "Exportar memoria"}
        </Button>
      </div>
      {error && <Notice tone="error">{error}</Notice>}
      {report && (
        <Notice>
          Exportado en {report.path} ({kb(report.bytes)}): {report.manifest.counts.memories} recuerdos, {report.manifest.counts.relations} relaciones,{" "}
          {report.manifest.counts.skills} habilidades. Modelo de embeddings: {report.manifest.embeddingModel}.
        </Notice>
      )}
    </section>
  );
}

function ImportForm({ onDone }: { onDone(): void }) {
  const [packages, setPackages] = useState<string[]>([]);
  const [path, setPath] = useState("");
  const [pass, setPass] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [conflict, setConflict] = useState<ConflictStrategy>("keep_local");
  const [skills, setSkills] = useState(true);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void backend().listMemoryPackages().then((p) => {
      setPackages(p);
      if (p[0]) setPath(p[0]);
    }).catch(() => undefined);
  }, []);

  const doPreview = async () => {
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      const p = await backend().previewImport(path, pass);
      setPreview(p);
      setSelected(new Set(p.memories.filter((m) => m.status !== "duplicate").map((m) => m.id)));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const apply = async () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      // Si no se desmarcó nada, se importa todo (también lo que no cupo en la lista de vista previa).
      const all = preview.memories.filter((m) => m.status !== "duplicate").every((m) => selected.has(m.id));
      setReport(await backend().applyImport(path, pass, { memoryIds: all ? undefined : [...selected], conflict, includeSkills: skills, includeExperiences: true, includeRelations: true }));
      setPreview(null);
      setPass("");
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="pane flex flex-col gap-3 p-5">
      <h2 className="text-sm font-semibold">Importar</h2>
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-mist">Archivo .jarvismem (ruta completa)</span>
        <input value={path} onChange={(e) => setPath(e.target.value)} list="jarvis-packages" className={input} aria-label="Ruta del paquete a importar" />
        <datalist id="jarvis-packages">{packages.map((p) => <option key={p} value={p} />)}</datalist>
      </label>
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-mist">Contraseña del paquete</span>
        <input type="password" autoComplete="off" value={pass} onChange={(e) => setPass(e.target.value)} className={input} />
      </label>
      <div>
        <Button disabled={!path || pass.length < 8 || busy} onClick={() => void doPreview()}>
          {busy && !preview ? "Verificando…" : "Revisar paquete"}
        </Button>
      </div>
      {error && <Notice tone="error">{error}</Notice>}

      {preview && (
        <div className="flex flex-col gap-3">
          <Notice>
            Exportado el {formatDate(preview.manifest.exportedAt)} {preview.sameInstallation ? "desde esta instalación" : "desde otra instalación"} · esquema v{preview.manifest.schemaVersion}.{" "}
            {preview.newMemories} nuevos, {preview.duplicates} duplicados, {preview.conflicts.length} en conflicto.
            {preview.needsReembed && " El modelo de embeddings es distinto: se recalcularán los vectores."}
          </Notice>
          {preview.conflicts.length > 0 && (
            <fieldset className="flex flex-col gap-1 text-sm">
              <legend className="mb-1 text-mist">Si un recuerdo existe con otro contenido</legend>
              {(Object.keys(CONFLICT_LABEL) as ConflictStrategy[]).map((c) => (
                <label key={c} className="flex items-center gap-2">
                  <input type="radio" name="conflict" checked={conflict === c} onChange={() => setConflict(c)} />
                  {CONFLICT_LABEL[c]}
                </label>
              ))}
            </fieldset>
          )}
          <ul className="max-h-56 overflow-y-auto rounded-lg border border-tide/60 text-sm">
            {preview.memories.map((m) => (
              <li key={m.id} className="flex items-center gap-2 border-b border-tide/40 px-2 py-1 last:border-0">
                <input type="checkbox" aria-label={`Importar ${m.summary}`} checked={selected.has(m.id)} disabled={m.status === "duplicate"} onChange={(e) => {
                  const next = new Set(selected);
                  if (e.target.checked) next.add(m.id); else next.delete(m.id);
                  setSelected(next);
                }} />
                <span className="min-w-0 flex-1 truncate text-ice">{m.summary}</span>
                <span className={`text-xs ${m.status === "new" ? "text-kelp" : m.status === "conflict" ? "text-amber" : "text-mist"}`}>
                  {m.status === "new" ? "nuevo" : m.status === "conflict" ? "conflicto" : "duplicado"}
                </span>
              </li>
            ))}
          </ul>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={skills} onChange={(e) => setSkills(e.target.checked)} />
            Importar habilidades ({preview.manifest.skills.length}). Llegan desactivadas hasta que las revises.
          </label>
          <div className="flex gap-2">
            <Button tone="primary" disabled={busy || selected.size === 0} onClick={() => void apply()}>
              {busy ? "Importando…" : `Importar ${selected.size} recuerdos`}
            </Button>
            <Button onClick={() => setPreview(null)}>Cancelar</Button>
          </div>
          <p className="text-xs text-mist">Antes de importar se crea una copia de seguridad automática.</p>
        </div>
      )}
      {report && (
        <Notice>
          Importados {report.imported}; duplicados omitidos {report.duplicatesSkipped}; conflictos: {report.conflictsKeptLocal} conservados, {report.conflictsOverwritten} reemplazados,{" "}
          {report.conflictsDuplicated} duplicados. {report.relations} relaciones, {report.reembedded} vectores recalculados, {report.skillsPendingReview} habilidades por revisar.
        </Notice>
      )}
    </section>
  );
}

function Backups({ onDone }: { onDone(): void }) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [msg, setMsg] = useState<{ tone: "info" | "error"; text: string } | null>(null);
  const load = () => void backend().listBackups().then(setBackups).catch(() => setBackups([]));
  useEffect(load, []);
  const act = async (fn: () => Promise<string>, ok: string) => {
    try {
      const path = await fn();
      setMsg({ tone: "info", text: `${ok} (${path})` });
      load();
      onDone();
    } catch (e) {
      setMsg({ tone: "error", text: e instanceof Error ? e.message : String(e) });
    }
  };
  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold">Copias de seguridad</h2>
        <Button onClick={() => void act(() => backend().createBackup(), "Copia creada")}>Crear copia ahora</Button>
      </div>
      {msg && <Notice tone={msg.tone}>{msg.text}</Notice>}
      {backups.length === 0 ? (
        <p className="text-sm text-mist">No hay copias. Se crea una automáticamente antes de cada importación.</p>
      ) : (
        <ul className="divide-y divide-tide/60 text-sm">
          {backups.map((b) => (
            <li key={b.name} className="flex items-center gap-3 py-2">
              <span className="flex-1 truncate font-mono text-xs text-ice">{b.name}</span>
              <span className="text-xs text-mist">{kb(b.bytes)}</span>
              {confirm === b.name ? (
                <>
                  <span className="text-xs text-amber">Reemplaza la memoria actual (antes se guarda otra copia).</span>
                  <Button tone="approve" onClick={() => { setConfirm(null); void act(() => backend().restoreBackup(b.name), "Restaurada; copia del estado anterior"); }}>Restaurar</Button>
                  <Button onClick={() => setConfirm(null)}>Cancelar</Button>
                </>
              ) : (
                <Button onClick={() => setConfirm(b.name)}>Restaurar…</Button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function TransferPanel({ exportsDir, onChanged }: { exportsDir: string; onChanged(): void }) {
  return (
    <div className="flex flex-col gap-6">
      {exportsDir && <p className="text-sm text-mist">Carpeta de paquetes: {exportsDir}</p>}
      <div className="grid grid-cols-2 gap-6">
        <ExportForm onDone={onChanged} />
        <ImportForm onDone={onChanged} />
      </div>
      <Backups onDone={onChanged} />
    </div>
  );
}
