// Piezas comunes de la pantalla de Configuración.
import type { ReactNode } from "react";

export const input = "w-full rounded-lg border border-tide bg-deep/70 px-3 py-1.5 text-sm focus:border-cyan focus:outline-none";

/** Campo con etiqueta. `group`: para grupos de botones (una <label> daría su texto al primer botón). */
export function Field({ label, hint, group, children }: { label: string; hint?: string; group?: boolean; children: ReactNode }) {
  const Tag = group ? "div" : "label";
  return (
    <Tag className="grid grid-cols-[200px_minmax(0,1fr)] items-start gap-4 py-2">
      <span className="pt-1.5 text-sm text-ice">
        {label}
        {hint && <span className="block text-xs text-mist">{hint}</span>}
      </span>
      <span>{children}</span>
    </Tag>
  );
}

/** Fila de solo lectura (estado). */
export function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[200px_minmax(0,1fr)] items-baseline gap-4 py-1.5 text-sm">
      <span className="text-mist">{label}</span>
      <span className="text-ice">{children}</span>
    </div>
  );
}

export function SectionTitle({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <header className="mb-3 flex items-end justify-between gap-4 border-b border-tide/60 pb-2">
      <h2 className="font-display text-xl font-semibold tracking-tight text-ice">{title}</h2>
      <div className="flex items-center gap-2">{children}</div>
    </header>
  );
}

export function relativeTime(iso: string | null): string {
  if (!iso) return "nunca";
  const seconds = Math.round((Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 60) return "hace unos segundos";
  if (seconds < 3600) return `hace ${Math.round(seconds / 60)} min`;
  if (seconds < 86_400) return `hace ${Math.round(seconds / 3600)} h`;
  return new Date(iso).toLocaleString("es");
}
