import { useEffect, useRef, useState } from "react";
import type { Conversation } from "@jarvis/shared-types";
import { backend } from "../../services/backend";
import { useAssistant, type StepView, type UiMessage } from "../../stores/assistantStore";
import { Button, Json, Notice, RiskTag, formatDate } from "../../components/ui";
import { PHASES, PHASE_LABEL } from "../../components/phases";
import { VoiceDock } from "../voice/VoiceDock";

const STEP_TEXT: Record<StepView["status"], string> = {
  pending: "Preparando",
  awaiting: "Espera tu autorización",
  running: "Ejecutando",
  done: "Hecho",
  failed: "Falló",
  denied: "No autorizada",
};

function StepCard({ step }: { step: StepView }) {
  const resolve = useAssistant((s) => s.resolveApproval);
  const [open, setOpen] = useState(false);
  const awaiting = step.status === "awaiting" && step.approvalId;
  return (
    <div className={`rounded-lg border px-3 py-2 text-sm ${awaiting ? "border-amber/60 bg-amber/5" : "border-tide bg-abyss/40"}`}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs text-ice">{step.tool}</span>
        <RiskTag risk={step.risk} />
        <span className={`text-xs ${step.status === "done" ? "text-kelp" : step.status === "failed" || step.status === "denied" ? "text-coral" : "text-mist"}`}>
          {STEP_TEXT[step.status]}
          {step.verification === "passed" && " · verificado"}
          {step.durationMs ? ` · ${step.durationMs} ms` : ""}
        </span>
        <button type="button" className="ml-auto text-xs text-mist hover:text-ice" onClick={() => setOpen(!open)} aria-expanded={open}>
          {open ? "Ocultar detalle" : "Ver detalle"}
        </button>
      </div>
      {awaiting && (
        <div className="mt-2 flex flex-col gap-2">
          <p className="text-amber">¿Autorizas esta acción? Motivo: {step.reason}.</p>
          <Json value={step.arguments} />
          <div className="flex gap-2">
            <Button tone="approve" onClick={() => void resolve(step.approvalId!, true)}>
              Autorizar
            </Button>
            <Button onClick={() => void resolve(step.approvalId!, false)}>Rechazar</Button>
          </div>
        </div>
      )}
      {step.status === "denied" && step.reason && <p className="mt-1 text-xs text-coral">{step.reason}</p>}
      {step.error && step.status === "failed" && <p className="mt-1 text-xs text-coral">{step.error}</p>}
      {open && (
        <div className="mt-2 grid gap-2">
          <Json value={step.arguments} />
          {step.output !== undefined && step.output !== null && <Json value={step.output} />}
        </div>
      )}
    </div>
  );
}

function Message({ m }: { m: UiMessage }) {
  if (m.role === "user") {
    return (
      <div className="flex justify-end">
        <p className="max-w-[70ch] rounded-2xl rounded-br-md bg-tide/70 px-4 py-2 whitespace-pre-wrap text-ice">{m.text}</p>
      </div>
    );
  }
  return (
    <div className="flex max-w-[72ch] flex-col gap-2">
      {m.steps.map((s) => (
        <StepCard key={s.callId} step={s} />
      ))}
      {m.text && (
        <p className="whitespace-pre-wrap text-ice">
          {m.text}
          {m.streaming && <span className="ml-0.5 inline-block h-4 w-1.5 translate-y-0.5 animate-pulse bg-cyan" aria-hidden />}
        </p>
      )}
      {!m.text && m.streaming && m.steps.length === 0 && <p className="text-mist">Pensando…</p>}
      {m.error && <Notice tone="error">{m.error.message}</Notice>}
      {m.cancelled && <p className="text-sm text-mist">Cancelaste esta tarea.</p>}
    </div>
  );
}

export function ChatScreen() {
  const { messages, activeRun, send, cancel, sendError, conversationId, openConversation, newConversation } = useAssistant();
  const [text, setText] = useState("");
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void backend().listConversations().then(setConversations).catch(() => setConversations([]));
  }, [activeRun === null, conversationId]);

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: "end" });
  }, [messages]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim() || activeRun) return;
    void send(text);
    setText("");
  };

  return (
    <div className="grid h-full grid-cols-[220px_minmax(0,1fr)_220px] gap-6">
      <aside className="flex min-h-0 flex-col gap-2">
        <Button tone="quiet" onClick={newConversation} disabled={activeRun !== null}>
          Nueva conversación
        </Button>
        <ul className="min-h-0 flex-1 overflow-y-auto">
          {conversations.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                disabled={activeRun !== null}
                onClick={() => void openConversation(c.id)}
                className={`w-full truncate rounded-md px-2 py-1.5 text-left text-sm ${c.id === conversationId ? "bg-deep text-ice" : "text-mist hover:text-ice"}`}
                title={`${c.title} · ${formatDate(c.updatedAt)}`}
              >
                {c.title}
              </button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex min-h-0 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto pr-2">
          <div className="flex flex-col gap-5 pb-4">
            {messages.length === 0 && (
              <p className="mt-16 text-center text-mist">
                Pide algo como «¿qué hora es?» o «¿cuánta memoria tiene esta PC?».
              </p>
            )}
            {messages.map((m) => (
              <Message key={m.id} m={m} />
            ))}
            <div ref={endRef} />
          </div>
        </div>
        {sendError && <Notice tone="error">{sendError}</Notice>}
        <form onSubmit={submit} className="mt-3 flex gap-2">
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) submit(e);
            }}
            rows={2}
            placeholder="Escribe a Jarvis… (Enter envía, Shift+Enter salta de línea)"
            aria-label="Mensaje"
            className="flex-1 resize-none rounded-xl border border-tide bg-deep/70 px-4 py-3 text-ice placeholder:text-mist/70 focus:border-cyan focus:outline-none"
          />
          {activeRun ? (
            <Button tone="danger" onClick={() => void cancel()}>
              Detener
            </Button>
          ) : (
            <Button tone="primary" type="submit" disabled={!text.trim()}>
              Enviar
            </Button>
          )}
        </form>
      </section>

      <aside className="flex flex-col gap-6">
        <div>
          <h2 className="mb-2 text-sm font-semibold">Ciclo actual</h2>
          <ol className="flex flex-col gap-1 text-sm">
            {PHASES.map((p) => {
              const seen = activeRun?.phasesSeen.includes(p);
              const current = activeRun?.phase === p;
              return (
                <li key={p} className={current ? "text-cyan" : seen ? "text-ice" : "text-mist/60"}>
                  {PHASE_LABEL[p]}
                </li>
              );
            })}
          </ol>
        </div>
        <div>
          <h2 className="mb-2 text-sm font-semibold">Voz</h2>
          <VoiceDock />
        </div>
      </aside>
    </div>
  );
}
