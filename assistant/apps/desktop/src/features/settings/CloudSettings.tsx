// Configuración → Infraestructura y Memoria: modo local o híbrido, emparejamiento con el Cloud Core,
// estado de la sincronización y conflictos.
import { useCallback, useEffect, useState } from "react";
import type { LocalConflict, SyncMode, SyncStatus } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { useUi } from "../../stores/uiStore";
import { Button, Dot, Notice } from "../../components/ui";
import { Field, input, relativeTime, Row, SectionTitle } from "./fields";

type Message = { tone: "info" | "error" | "warn"; text: string } | null;

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

function conflictText(c: LocalConflict, side: "server" | "device"): string {
  const data = side === "server" ? c.serverData : c.deviceData;
  if (!data) return side === "server" ? "(borrado en el Cloud)" : "(borrado en este equipo)";
  const content = data["content"] ?? data["procedure"] ?? data["name"];
  return typeof content === "string" ? content : JSON.stringify(data);
}

export function ConflictList({ conflicts, onResolved }: { conflicts: LocalConflict[]; onResolved: (s: SyncStatus) => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [edit, setEdit] = useState<Record<string, string>>({});

  const resolve = async (c: LocalConflict, resolution: "keep_server" | "keep_device" | "custom") => {
    setBusy(c.id);
    setError(null);
    try {
      const data = resolution === "custom" && c.deviceData ? { ...c.deviceData, content: edit[c.id] ?? conflictText(c, "device") } : undefined;
      onResolved(await backend().cloudResolveConflict(c.id, resolution, data));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  if (conflicts.length === 0) return <p className="py-2 text-sm text-mist">Sin conflictos.</p>;
  return (
    <ul className="flex flex-col gap-3" aria-label="Conflictos de sincronización">
      {error && <Notice tone="error">{error}</Notice>}
      {conflicts.map((c) => (
        <li key={c.id} className="rounded-xl border border-amber/40 bg-amber/5 p-3 text-sm">
          <p className="mb-2 text-xs text-amber">
            {c.entity === "memory" ? "Recuerdo" : c.entity} editado en dos equipos
            {c.fields.length > 0 && ` · campos: ${c.fields.join(", ")}`}
          </p>
          <div className="grid gap-2 md:grid-cols-2">
            <div className="rounded-lg border border-tide p-2">
              <p className="text-xs text-mist">En el Cloud (v{c.serverVersion})</p>
              <p className="text-ice">{conflictText(c, "server")}</p>
            </div>
            <div className="rounded-lg border border-tide p-2">
              <p className="text-xs text-mist">En este equipo</p>
              <p className="text-ice">{conflictText(c, "device")}</p>
            </div>
          </div>
          {c.entity === "memory" && c.deviceData && (
            <textarea
              aria-label="Versión combinada"
              className={`${input} mt-2`}
              rows={2}
              value={edit[c.id] ?? conflictText(c, "device")}
              onChange={(e) => setEdit({ ...edit, [c.id]: e.target.value })}
            />
          )}
          <div className="mt-2 flex flex-wrap gap-2">
            <Button disabled={busy === c.id} onClick={() => void resolve(c, "keep_server")}>
              Quedarme con la del Cloud
            </Button>
            <Button disabled={busy === c.id} onClick={() => void resolve(c, "keep_device")}>
              Quedarme con la de este equipo
            </Button>
            {c.entity === "memory" && c.deviceData && (
              <Button tone="primary" disabled={busy === c.id} onClick={() => void resolve(c, "custom")}>
                Guardar la versión combinada
              </Button>
            )}
          </div>
        </li>
      ))}
    </ul>
  );
}

export function CloudSettings() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [conflicts, setConflicts] = useState<LocalConflict[]>([]);
  const [message, setMessage] = useState<Message>(null);
  const [busy, setBusy] = useState(false);
  const [form, setForm] = useState({ url: "", email: "", password: "", deviceName: "PC de escritorio" });
  const go = useUi((s) => s.go);

  const refresh = useCallback(async () => {
    try {
      const [s, c] = await Promise.all([backend().cloudStatus(), backend().cloudConflicts()]);
      setStatus(s);
      setConflicts(c);
    } catch (e) {
      setMessage({ tone: "error", text: errorText(e) });
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 10_000);
    return () => clearInterval(timer);
  }, [refresh]);

  const run = async (label: string, action: () => Promise<unknown>) => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await action();
      await refresh();
      setMessage({ tone: "info", text: typeof result === "string" ? result : label });
    } catch (e) {
      setMessage({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  if (!status) return <p className="text-mist">Cargando…</p>;
  const setMode = (mode: SyncMode) => run(mode === "hybrid" ? "Modo híbrido activado." : "Modo local: nada sale de este equipo.", () => backend().cloudSetMode(mode));

  return (
    <>
      <section aria-labelledby="infra">
        <SectionTitle title="Infraestructura" />
        <span id="infra" className="sr-only">Infraestructura</span>
        {message && <Notice tone={message.tone}>{message.text}</Notice>}
        <Field label="Modo" group hint="Local: todo en esta PC. Híbrido: local primero y sincronización con tu Cloud Core">
          <span className="flex gap-2 pt-0.5" role="radiogroup" aria-label="Modo de funcionamiento">
            {(["local", "hybrid"] as SyncMode[]).map((m) => (
              <button
                key={m}
                type="button"
                role="radio"
                aria-checked={status.mode === m}
                disabled={busy || (m === "hybrid" && !status.paired)}
                onClick={() => void setMode(m)}
                className={`rounded-lg border px-3 py-1.5 text-sm ${status.mode === m ? "border-cyan bg-cyan/10 text-ice" : "border-tide text-mist hover:text-ice"} disabled:opacity-40`}
              >
                {m === "local" ? "Local" : "Híbrido"}
              </button>
            ))}
          </span>
        </Field>

        {status.paired ? (
          <>
            <Row label="Cloud Core">
              <span className="flex items-center gap-2">
                <Dot ok={status.online} pending={!status.online && !status.lastError} />
                {status.baseUrl}
                {status.latencyMs !== null && <span className="text-mist">· {status.latencyMs} ms</span>}
              </span>
            </Row>
            <Row label="Este dispositivo">
              {status.deviceName} <span className="text-xs text-mist">({status.deviceId?.slice(0, 8)}…)</span>
            </Row>
            <Row label="Última sincronización">{status.syncing ? "sincronizando…" : relativeTime(status.lastSyncAt)}</Row>
            <Row label="Búsqueda en el Cloud">{status.vectorSearch === "pgvector" ? "por significado (pgvector)" : status.vectorSearch ?? "sin comprobar"}</Row>
            <Row label="Credencial guardada en">{status.vault}</Row>
            {status.lastError && <Notice tone="warn">{status.lastError}</Notice>}
            <div className="mt-3 flex flex-wrap gap-2">
              <Button disabled={busy} onClick={() => void run("Cloud Core accesible.", async () => {
                const p = await backend().cloudProbe();
                return `Cloud Core accesible (${p.latencyMs} ms).`;
              })}>
                Comprobar conexión
              </Button>
              <Button tone="danger" disabled={busy} onClick={() => void run("Este equipo ya no sincroniza. Para invalidar su credencial, revócalo en el Cloud Core.", () => backend().cloudUnpair())}>
                Desemparejar
              </Button>
            </div>
          </>
        ) : (
          <form
            className="mt-2 rounded-xl border border-tide p-4"
            aria-label="Emparejar con el Cloud Core"
            onSubmit={(e) => {
              e.preventDefault();
              void run("Equipo emparejado: la sincronización empieza ahora.", async () => {
                await backend().cloudPair(form);
                setForm({ ...form, password: "" });
              });
            }}
          >
            <p className="mb-2 text-sm text-mist">
              Conecta este equipo a tu Cloud Core para que la memoria, las experiencias y las habilidades sobrevivan a un cambio
              de PC. La contraseña solo se usa para registrar el equipo; no se guarda.
            </p>
            <Field label="URL del Cloud Core" hint="https://jarvis.tu-dominio">
              <input className={input} required value={form.url} onChange={(e) => setForm({ ...form, url: e.target.value })} />
            </Field>
            <Field label="Correo del propietario">
              <input className={input} type="email" required autoComplete="username" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} />
            </Field>
            <Field label="Contraseña">
              <input className={input} type="password" required autoComplete="current-password" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} />
            </Field>
            <Field label="Nombre de este equipo">
              <input className={input} required maxLength={64} value={form.deviceName} onChange={(e) => setForm({ ...form, deviceName: e.target.value })} />
            </Field>
            <div className="flex gap-2">
              <Button tone="primary" type="submit" disabled={busy}>
                Emparejar
              </Button>
              <Button disabled={busy || !form.url} onClick={() => void run("", async () => {
                const p = await backend().cloudProbe(form.url);
                return `Cloud Core accesible (${p.latencyMs} ms).`;
              })}>
                Probar URL
              </Button>
            </div>
          </form>
        )}
      </section>

      <section aria-label="Memoria y sincronización">
        <SectionTitle title="Memoria">
          <Button disabled={busy || !status.paired || status.mode !== "hybrid"} onClick={() => void run("", async () => {
            const r = await backend().cloudSyncNow();
            return `Sincronizado: ${r.pushed} enviados, ${r.pulled} recibidos${r.merged ? `, ${r.merged} fusionados` : ""}${r.conflicts ? `, ${r.conflicts} en conflicto` : ""}.`;
          })}>
            Sincronizar ahora
          </Button>
        </SectionTitle>
        <Row label="Recuerdos en este equipo">{status.localMemories}</Row>
        <Row label="Sincronizados">{status.paired ? `${status.syncedMemories} de ${status.localMemories}` : "— (modo local)"}</Row>
        <Row label="Estado">
          {!status.paired
            ? "Solo en este equipo"
            : status.pending > 0
              ? `${status.pending} ${status.pending === 1 ? "cambio pendiente" : "cambios pendientes"} de enviar${status.online ? "" : " (se enviarán al volver la conexión)"}`
              : "Al día"}
          {status.rejected > 0 && <span className="ml-2 text-coral">· {status.rejected} rechazados por el Cloud</span>}
        </Row>
        <Row label="Conflictos">{conflicts.length === 0 ? "ninguno" : <span className="text-amber">{conflicts.length} por resolver</span>}</Row>
        {conflicts.length > 0 && <ConflictList conflicts={conflicts} onResolved={(s) => { setStatus(s); void refresh(); }} />}
        <div className="mt-3 flex gap-2">
          <Button onClick={() => go("memory")}>Exportar o importar memoria…</Button>
        </div>
      </section>
    </>
  );
}
