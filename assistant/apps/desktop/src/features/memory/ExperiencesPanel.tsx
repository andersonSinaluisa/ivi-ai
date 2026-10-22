// Experiencias registradas y habilidades aprendidas (con versiones y revisión de las importadas).
import { useCallback, useEffect, useState } from "react";
import type { Experience, Skill, SkillVersion } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { Button, Empty, Json, Notice, formatDate } from "../../components/ui";

function SkillCard({ skill, onChanged }: { skill: Skill; onChanged(): void }) {
  const [editing, setEditing] = useState(false);
  const [procedure, setProcedure] = useState(skill.procedure);
  const [versions, setVersions] = useState<SkillVersion[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const act = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <li className="flex flex-col gap-2 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <p className="text-ice">{skill.name}</p>
        <span className="text-xs text-mist">v{skill.currentVersion} · {skill.source === "import" ? "importada" : "aprendida"}</span>
        {skill.requiresReview && <span className="text-xs text-amber">Pendiente de revisión</span>}
        <label className="ml-auto flex items-center gap-2 text-sm text-mist">
          <input type="checkbox" checked={skill.enabled} onChange={(e) => void act(() => backend().setSkillEnabled(skill.id, e.target.checked))} />
          {skill.requiresReview ? "Revisé y la activo" : "Activa"}
        </label>
      </div>
      {editing ? (
        <div className="flex flex-col gap-2">
          <textarea rows={4} value={procedure} onChange={(e) => setProcedure(e.target.value)} aria-label="Procedimiento" className="rounded-lg border border-cyan bg-deep/70 px-3 py-2 font-mono text-xs" />
          <div className="flex gap-2">
            <Button tone="primary" onClick={() => void act(async () => { await backend().updateSkill(skill.id, procedure, "editada a mano"); setEditing(false); })}>
              Guardar como v{skill.currentVersion + 1}
            </Button>
            <Button onClick={() => setEditing(false)}>Cancelar</Button>
          </div>
        </div>
      ) : (
        <pre className="rounded-md bg-abyss/70 p-2 font-mono text-xs whitespace-pre-wrap text-mist">{skill.procedure}</pre>
      )}
      <div className="flex gap-3 text-xs">
        {!editing && <button type="button" className="text-cyan hover:underline" onClick={() => setEditing(true)}>Corregir procedimiento</button>}
        <button type="button" className="text-mist hover:text-ice" onClick={() => void backend().skillVersions(skill.id).then(setVersions)}>
          Ver versiones
        </button>
      </div>
      {versions && (
        <ul className="text-xs text-mist">
          {versions.map((v) => (
            <li key={v.version}>v{v.version} · {formatDate(v.createdAt)}{v.notes ? ` · ${v.notes}` : ""}</li>
          ))}
        </ul>
      )}
      {error && <Notice tone="error">{error}</Notice>}
    </li>
  );
}

export function ExperiencesPanel() {
  const [experiences, setExperiences] = useState<Experience[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    void backend().listExperiences().then(setExperiences).catch((e: Error) => setError(e.message));
    void backend().listSkills().then(setSkills).catch((e: Error) => setError(e.message));
  }, []);
  useEffect(load, [load]);

  return (
    <div className="grid grid-cols-2 gap-8">
      <section>
        <h2 className="mb-1 text-sm font-semibold">Habilidades aprendidas</h2>
        <p className="mb-3 text-sm text-mist">Nacen solo de tareas verificadas y completadas. Jarvis no las ejecuta por su cuenta.</p>
        {skills.length === 0 ? <Empty>Aún no hay habilidades.</Empty> : (
          <ul className="divide-y divide-tide/60">
            {skills.map((s) => <SkillCard key={s.id} skill={s} onChanged={load} />)}
          </ul>
        )}
      </section>
      <section>
        <h2 className="mb-1 text-sm font-semibold">Experiencias</h2>
        <p className="mb-3 text-sm text-mist">Cada tarea con herramientas. Las repetidas se agrupan.</p>
        {error && <Notice tone="error">{error}</Notice>}
        {experiences.length === 0 ? <Empty>Aún no hay experiencias.</Empty> : (
          <ul className="divide-y divide-tide/60">
            {experiences.map((x) => {
              const promotable = x.verified && x.completed && !x.promoted;
              return (
                <li key={x.id} className="flex flex-col gap-2 py-3">
                  <div className="flex flex-wrap items-center gap-3">
                    <p className="text-ice">{x.goal}</p>
                    <span className={`text-xs ${x.verified ? "text-kelp" : "text-mist"}`}>{x.verified ? "Verificada" : "Sin verificar"}</span>
                    {!x.completed && <span className="text-xs text-amber">Incompleta</span>}
                    {x.occurrences > 1 && <span className="text-xs text-mist">{x.occurrences} veces</span>}
                    {x.promoted && <span className="text-xs text-cyan">Es habilidad</span>}
                    <span className="text-xs text-mist">{formatDate(x.createdAt)}</span>
                    {promotable && (
                      <Button className="ml-auto" onClick={() => void backend().promoteExperience(x.id).then(load).catch((e: Error) => setError(e.message))}>
                        Guardar como habilidad
                      </Button>
                    )}
                  </div>
                  <Json value={x.steps.map((s) => ({ herramienta: s.tool, ok: s.ok, verificacion: s.verification }))} />
                  {x.errors.length > 0 && <p className="text-xs text-coral">{x.errors.join(" · ")}</p>}
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}
