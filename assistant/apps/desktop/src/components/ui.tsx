// Piezas pequeñas y reutilizables.
import type { ReactNode } from "react";
import type { RiskClass } from "@jarvis/shared-types";

export const RISK_LABEL: Record<RiskClass, string> = {
  READ_ONLY: "Solo lectura",
  LOW_RISK: "Riesgo bajo",
  SENSITIVE: "Sensible",
  DESTRUCTIVE: "Destructiva",
};

const RISK_CLASS: Record<RiskClass, string> = {
  READ_ONLY: "text-mist border-tide",
  LOW_RISK: "text-cyan-soft border-current",
  SENSITIVE: "text-amber border-amber/50",
  DESTRUCTIVE: "text-coral border-coral/50",
};

export function RiskTag({ risk }: { risk: RiskClass }) {
  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs ${RISK_CLASS[risk]}`}>
      {RISK_LABEL[risk]}
    </span>
  );
}

export function Dot({ ok, pending }: { ok: boolean; pending?: boolean }) {
  const cls = pending ? "bg-mist" : ok ? "bg-kelp" : "bg-coral";
  return <span aria-hidden className={`inline-block h-2 w-2 shrink-0 rounded-full ${cls}`} />;
}

export function ScreenHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <header className="mb-6 flex items-end justify-between gap-4">
      <h1 className="font-display text-2xl font-semibold tracking-tight text-ice">{title}</h1>
      <div className="flex items-center gap-2">{children}</div>
    </header>
  );
}

type ButtonTone = "primary" | "quiet" | "danger" | "approve";
const TONE: Record<ButtonTone, string> = {
  primary: "bg-cyan text-abyss hover:bg-cyan-soft",
  quiet: "border border-tide text-ice hover:border-current hover:bg-deep",
  danger: "border border-coral/50 text-coral hover:bg-coral/10",
  approve: "bg-amber text-abyss hover:brightness-110",
};

export function Button({
  tone = "quiet",
  className = "",
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { tone?: ButtonTone }) {
  return (
    <button
      type="button"
      className={`rounded-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${TONE[tone]} ${className}`}
      {...rest}
    />
  );
}

export function Notice({ tone = "info", children }: { tone?: "info" | "error" | "warn"; children: ReactNode }) {
  const cls =
    tone === "error"
      ? "border-coral/40 text-coral"
      : tone === "warn"
        ? "border-amber/40 text-amber"
        : "border-tide text-mist";
  return <div className={`rounded-lg border px-3 py-2 text-sm ${cls}`}>{children}</div>;
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="py-10 text-center text-sm text-mist">{children}</p>;
}

export function formatDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("es-EC", { dateStyle: "short", timeStyle: "short" });
}

export function Json({ value }: { value: unknown }) {
  return (
    <pre className="max-h-48 overflow-auto rounded-md bg-abyss/70 p-2 font-mono text-xs leading-relaxed text-mist">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}
