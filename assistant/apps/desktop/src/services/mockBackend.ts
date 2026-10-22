// Backend simulado para la vista previa en el navegador y las pruebas de interfaz.
// No se usa dentro de Tauri. Los recuerdos de demostración son genéricos y se indican como tales.
import type {
  AgentEvent,
  BackendVisualEvent,
  Experience,
  GraphEdge,
  GraphNode,
  MemoryKind,
  MemoryRecord,
  MemoryRelation,
  RelationKind,
  InferenceView,
  LocalConflict,
  Settings,
  Skill,
  SyncStatus,
} from "@jarvis/shared-types";
import type { Backend } from "./backend";

function rng(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const ENTITIES = ["Proyecto Atlas", "Proyecto Faro", "Servidor VPS", "Python", "React", "Ollama", "Casa", "Trabajo", "Viajes", "Música", "Lecturas", "Huerto"];
const FACTS = [
  "usa una base de datos PostgreSQL",
  "se despliega con Docker Compose",
  "tiene pruebas automáticas en cada cambio",
  "prefiere respuestas cortas sobre este tema",
  "tiene una reunión los lunes",
  "necesita revisar los permisos",
  "guarda copias de seguridad cada noche",
  "usa la paleta azul y cian",
  "se documenta en Markdown",
  "depende de una API externa",
];

function makeId(r: () => number): string {
  const h = () => Math.floor(r() * 0xffff).toString(16).padStart(4, "0");
  return `${h()}${h()}-${h()}-4${h().slice(1)}-a${h().slice(1)}-${h()}${h()}${h()}`;
}

function demoMemory(id: string, kind: MemoryKind, content: string, r: () => number, verified: boolean): MemoryRecord {
  const when = new Date(Date.now() - Math.floor(r() * 60) * 86_400_000).toISOString();
  return {
    id,
    kind,
    content,
    summary: content.slice(0, 80),
    createdAt: when,
    updatedAt: when,
    source: verified ? "usuario" : "Jarvis (conversación)",
    sourceId: null,
    relevance: 0.5,
    confidence: verified ? 1 : 0.6 + r() * 0.3,
    verification: verified ? "verified" : "unverified",
    tags: ["demo"],
    references: [],
    retention: { kind: "permanent" },
    contentHash: id,
  };
}

function buildDemo(): { memories: MemoryRecord[]; relations: MemoryRelation[] } {
  const r = rng(42);
  const memories: MemoryRecord[] = [];
  const relations: MemoryRelation[] = [];
  const rel = (a: string, b: string, kind: RelationKind, provenance: MemoryRelation["provenance"], confidence: number) =>
    relations.push({
      id: makeId(r),
      sourceId: a,
      targetId: b,
      kind,
      confidence,
      provenance,
      verified: provenance === "user",
      note: null,
      createdAt: new Date().toISOString(),
    });
  const entityIds = ENTITIES.map((name) => {
    const m = demoMemory(makeId(r), "entity", `${name} (demostración)`, r, true);
    memories.push(m);
    return m.id;
  });
  for (let i = 0; i < 110; i++) {
    const e = Math.floor(r() * ENTITIES.length);
    const m = demoMemory(makeId(r), "semantic", `${ENTITIES[e]} ${FACTS[Math.floor(r() * FACTS.length)]} (demo ${i + 1})`, r, r() > 0.4);
    memories.push(m);
    rel(m.id, entityIds[e], "BELONGS_TO", "user", 1);
    if (r() > 0.6 && memories.length > 20) {
      const other = memories[12 + Math.floor(r() * (memories.length - 13))];
      if (other.id !== m.id) rel(m.id, other.id, "RELATED_TO", "inferred", 0.4 + r() * 0.5);
    }
  }
  for (let i = 0; i < 14; i++) {
    const e = Math.floor(r() * ENTITIES.length);
    const m = demoMemory(makeId(r), "procedural", `Cómo publicar ${ENTITIES[e]}: probar, empaquetar y desplegar (demo)`, r, true);
    memories.push(m);
    rel(m.id, entityIds[e], "USED_IN", "user", 1);
  }
  for (let i = 0; i < 12; i++) {
    const e = Math.floor(r() * ENTITIES.length);
    const m = demoMemory(makeId(r), "experience", `Tarea resuelta en ${ENTITIES[e]} (demo)`, r, r() > 0.5);
    memories.push(m);
    rel(m.id, entityIds[e], "SOLVED_BY", "agent", 0.7);
  }
  return { memories, relations };
}

function toNode(m: MemoryRecord, relations: MemoryRelation[]): GraphNode {
  return {
    id: m.id,
    kind: m.kind,
    label: m.summary || m.content.slice(0, 80),
    confidence: m.confidence,
    verification: m.verification,
    degree: relations.filter((r) => r.sourceId === m.id || r.targetId === m.id).length,
    createdAt: m.createdAt,
  };
}

function toEdge(r: MemoryRelation): GraphEdge {
  return { id: r.id, source: r.sourceId, target: r.targetId, kind: r.kind, confidence: r.confidence, provenance: r.provenance, verified: r.verified };
}

export function createMockBackend(delayMs = 30, opts: { demoGraph?: boolean } = {}): Backend {
  const sleep = (ms: number) => new Promise((res) => setTimeout(res, ms));
  const approvals = new Map<string, (ok: boolean) => void>();
  const listeners = new Set<(e: BackendVisualEvent) => void>();
  const emit = (e: BackendVisualEvent) => listeners.forEach((l) => l(e));
  const demo = opts.demoGraph === false ? { memories: [], relations: [] } : buildDemo();
  const memories: MemoryRecord[] = demo.memories;
  const relations: MemoryRelation[] = demo.relations;
  const experiences: Experience[] = [];
  const skills: Skill[] = [];
  let settings: Settings = {
    model: "qwen3:4b",
    baseUrl: "http://127.0.0.1:11434",
    temperature: 0.4,
    numCtx: 8192,
    think: false,
    maxSteps: 6,
    autoApprove: ["READ_ONLY", "LOW_RISK"],
  };
  let n = 0;
  const id = () => `mock-${++n}`;
  const desktopOnly = async () => {
    throw new Error("Disponible solo en la app de escritorio (vista previa sin backend).");
  };

  async function simulate(runId: string, conversationId: string, text: string, send: (e: AgentEvent) => void) {
    send({ type: "run_started", runId, conversationId });
    emit({ type: "TASK_STARTED", runId, goal: text });
    for (const phase of ["observe", "understand"] as const) {
      send({ type: "phase", runId, phase });
      await sleep(delayMs);
    }
    // Recuperación simulada: recuerdos que comparten alguna palabra.
    const words = text.toLowerCase().split(/\W+/).filter((w) => w.length > 3);
    const hits = memories.filter((m) => words.some((w) => m.content.toLowerCase().includes(w))).slice(0, 5);
    if (hits.length) {
      const ids = hits.map((m) => m.id);
      emit({
        type: "MEMORY_RETRIEVED",
        runId,
        memoryIds: ids,
        relationIds: relations.filter((r) => ids.includes(r.sourceId)).map((r) => r.id),
      });
    }
    send({ type: "phase", runId, phase: "plan" });
    await sleep(delayMs);
    if (/cierra|cerrar/i.test(text)) {
      const callId = id();
      const approvalId = id();
      send({ type: "tool_call", runId, callId, tool: "close_app", arguments: { appId: "notepad" }, risk: "SENSITIVE" });
      send({ type: "phase", runId, phase: "authorize" });
      send({ type: "approval_required", runId, approvalId, callId, tool: "close_app", arguments: { appId: "notepad" }, risk: "SENSITIVE", reason: "acción de riesgo sensible" });
      const ok = await new Promise<boolean>((resolve) => approvals.set(approvalId, resolve));
      send({ type: "approval_resolved", runId, approvalId, approved: ok, reason: ok ? null : "el usuario la rechazó" });
      if (ok) {
        send({ type: "phase", runId, phase: "execute" });
        await sleep(delayMs);
        send({ type: "phase", runId, phase: "verify" });
      }
      send({
        type: "tool_result",
        runId,
        callId,
        tool: "close_app",
        ok,
        output: ok ? { closed: true } : null,
        error: ok ? null : "Acción no autorizada",
        verification: ok ? "passed" : "not_applicable",
        durationMs: ok ? 120 : 0,
      });
    }
    const reply = /cierra|cerrar/i.test(text)
      ? "Listo, cerré el Bloc de notas."
      : hits.length
        ? `Encontré ${hits.length} recuerdos relacionados (vista previa).`
        : "Modo vista previa: no hay backend de Tauri ni Ollama conectados.";
    for (const word of reply.split(/(?<= )/)) {
      send({ type: "token", runId, text: word });
      await sleep(delayMs / 3);
    }
    send({ type: "phase", runId, phase: "remember" });
    send({ type: "final", runId, text: reply });
    emit({ type: "TASK_COMPLETED", runId, status: "completed" });
  }

  // Cloud Core simulado: empieza en modo local y sin emparejar.
  let cloud: SyncStatus = {
    mode: "local",
    paired: false,
    baseUrl: null,
    deviceId: null,
    deviceName: null,
    vault: "vista previa (sin almacén real)",
    online: false,
    latencyMs: null,
    vectorSearch: null,
    lastSyncAt: null,
    lastError: null,
    pending: 0,
    rejected: 0,
    conflicts: 0,
    cursor: 0,
    syncing: false,
    localMemories: memories.length,
    syncedMemories: 0,
  };
  let conflicts: LocalConflict[] = [];
  let inference: InferenceView = {
    settings: { mode: "local", remoteKind: "ollama", remoteUrl: "", remoteModel: "" },
    hasApiKey: false,
    status: { mode: "local", remoteConfigured: false, remoteName: null, lastProvider: null, lastFallback: null },
    vault: "vista previa (sin almacén real)",
  };

  const backend: Backend = {
    kind: "mock",
    async sendMessage(conversationId, text, onEvent) {
      const info = { runId: id(), conversationId: conversationId ?? id() };
      void simulate(info.runId, info.conversationId, text, onEvent);
      return info;
    },
    cancelRun: async () => false,
    async resolveApproval(approvalId, approved) {
      const resolve = approvals.get(approvalId);
      approvals.delete(approvalId);
      resolve?.(approved);
      return Boolean(resolve);
    },
    listConversations: async () => [],
    getMessages: async () => [],
    deleteConversation: async () => true,
    getStatus: async () => ({
      ollama: { ok: false, detail: "Vista previa sin backend", version: null },
      model: { ok: false, detail: "sin comprobar", name: settings.model, installed: false, alternatives: [] },
      windowsAgent: { ok: false, detail: "sin comprobar", toolCount: 0 },
      database: { ok: true, detail: "simulada" },
      localTools: 4,
    }),
    listModels: async () => ["qwen3:4b"],
    listAudit: async () => [],
    listRuns: async () => [],
    listTools: async () => [
      {
        id: "local.get_current_time",
        name: "get_current_time",
        description: "Devuelve la fecha y hora actuales.",
        risk: "READ_ONLY",
        permissions: ["system.clock.read"],
        timeoutMs: 1000,
        cancellable: false,
        source: "local",
        parameters: { type: "object", properties: {} },
        policyMode: "default",
        effectiveDecision: "allow",
      },
    ],
    setToolMode: async () => undefined,
    refreshWindowsAgent: async () => ({ ok: false, detail: "vista previa", toolCount: 0 }),
    getSettings: async () => settings,
    async updateSettings(s) {
      settings = s;
      return s;
    },
    voiceConnection: async () => ({ baseUrl: "http://127.0.0.1:8765", token: null, error: "Vista previa sin backend" }),

    async listMemories(filter, offset, limit) {
      const q = filter.query?.toLowerCase();
      const items = memories
        .filter((m) => (!filter.kind || m.kind === filter.kind) && (!filter.verification || m.verification === filter.verification))
        .filter((m) => !q || m.content.toLowerCase().includes(q))
        .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
      return { items: items.slice(offset, offset + limit), total: items.length, offset, limit };
    },
    async getMemory(mid) {
      const memory = memories.find((m) => m.id === mid);
      if (!memory) throw new Error("recuerdo no encontrado");
      const rels = relations.filter((r) => r.sourceId === mid || r.targetId === mid);
      return {
        memory,
        source: { id: "s", kind: memory.verification === "verified" ? "user" : "agent", reference: null, label: memory.source, createdAt: memory.createdAt },
        relations: rels.flatMap((r) => {
          const otherId = r.sourceId === mid ? r.targetId : r.sourceId;
          const other = memories.find((m) => m.id === otherId);
          return other ? [{ relation: r, other: toNode(other, relations), outgoing: r.sourceId === mid }] : [];
        }),
        events: [],
      };
    },
    async addMemory(input) {
      const m = demoMemory(`${Date.now().toString(16)}-0000-4000-a000-${String(++n).padStart(12, "0")}`, input.kind ?? "semantic", input.content, Math.random, true);
      m.tags = input.tags ?? [];
      m.createdAt = m.updatedAt = new Date().toISOString();
      memories.unshift(m);
      emit({ type: "MEMORY_CREATED", node: toNode(m, relations), edges: [] });
      return { memory: m, created: true, relations: [], embeddingPending: false };
    },
    async updateMemory(mid, patch) {
      const m = memories.find((x) => x.id === mid);
      if (!m) throw new Error("recuerdo no encontrado");
      Object.assign(m, patch, { updatedAt: new Date().toISOString() });
      emit({ type: "MEMORY_UPDATED", node: toNode(m, relations) });
      return m;
    },
    async setMemoryVerification(mid, verification) {
      return backend.updateMemory(mid, { verification });
    },
    async deleteMemory(mid) {
      const i = memories.findIndex((x) => x.id === mid);
      if (i >= 0) memories.splice(i, 1);
      for (let j = relations.length - 1; j >= 0; j--) {
        if (relations[j].sourceId === mid || relations[j].targetId === mid) relations.splice(j, 1);
      }
      emit({ type: "MEMORY_DELETED", memoryId: mid });
    },
    async addRelation(sourceId, targetId, kind) {
      const r: MemoryRelation = { id: id(), sourceId, targetId, kind, confidence: 1, provenance: "user", verified: true, note: null, createdAt: new Date().toISOString() };
      relations.push(r);
      emit({ type: "MEMORY_CONNECTED", edge: toEdge(r) });
      return r;
    },
    async verifyRelation(rid, verified) {
      const r = relations.find((x) => x.id === rid);
      if (!r) throw new Error("relación no encontrada");
      r.verified = verified;
      return r;
    },
    async deleteRelation(rid) {
      const i = relations.findIndex((x) => x.id === rid);
      if (i >= 0) relations.splice(i, 1);
    },
    async memoryGraph(q) {
      let ids: Set<string>;
      if (q.focus) {
        ids = new Set([q.focus]);
        for (let d = 0; d < (q.depth ?? 2); d++) {
          for (const r of relations) {
            if (ids.has(r.sourceId)) ids.add(r.targetId);
            else if (ids.has(r.targetId)) ids.add(r.sourceId);
          }
        }
      } else {
        ids = new Set(memories.slice(q.offset ?? 0, (q.offset ?? 0) + (q.limit ?? 400)).map((m) => m.id));
      }
      const nodes = memories.filter((m) => ids.has(m.id)).map((m) => toNode(m, relations));
      const edges = relations.filter((r) => ids.has(r.sourceId) && ids.has(r.targetId)).map(toEdge);
      return { nodes, edges, totalMemories: memories.length, truncated: nodes.length < memories.length };
    },
    async searchMemories(query) {
      const page = await backend.listMemories({ query }, 0, 10);
      return {
        intent: { kind: "recall", keywords: query.split(/\s+/) },
        items: page.items.map((m) => ({ memory: m, score: 0.5, similarity: 0.5, via: { type: "direct" } })),
        context: "",
        tokensEstimate: 0,
        relationIds: [],
        embeddingModel: "demo",
        semantic: false,
        degraded: false,
      };
    },
    memoryStats: async () => ({
      total: memories.length,
      byKind: memories.reduce<Record<string, number>>((acc, m) => ({ ...acc, [m.kind]: (acc[m.kind] ?? 0) + 1 }), {}),
      relations: relations.length,
      skills: skills.length,
      embedding: { model: "demo", semantic: false, detail: "vista previa: datos de demostración", pending: 0 },
      exportsDir: "Documentos\\Jarvis",
    }),
    consolidateMemory: async () => ({ removedExpired: 0, mergedDuplicates: 0, prunedRelations: 0 }),
    refreshEmbeddings: async () => "vista previa",
    listExperiences: async () => experiences,
    promoteExperience: desktopOnly,
    listSkills: async () => skills,
    skillVersions: async () => [],
    setSkillEnabled: desktopOnly,
    updateSkill: desktopOnly,
    defaultExportPath: async () => "C:\\Users\\tu-usuario\\Documents\\Jarvis\\jarvis-memory.jarvismem",
    listMemoryPackages: async () => [],
    exportMemory: desktopOnly,
    previewImport: desktopOnly,
    applyImport: desktopOnly,
    listBackups: async () => [],
    createBackup: desktopOnly,
    restoreBackup: desktopOnly,
    cloudStatus: async () => ({ ...cloud, localMemories: memories.length, conflicts: conflicts.length }),
    async cloudProbe(url) {
      await sleep(delayMs);
      if (!url && !cloud.baseUrl) throw new Error("Cloud Core sin configurar");
      cloud = { ...cloud, online: true, latencyMs: 48, vectorSearch: "pgvector" };
      return { latencyMs: 48, vectorSearch: "pgvector" };
    },
    async cloudPair(request) {
      await sleep(delayMs);
      if (!/^https:\/\//.test(request.url) && !/^http:\/\/(localhost|127\.0\.0\.1)/.test(request.url)) {
        throw new Error("configuración: el Cloud Core requiere HTTPS (HTTP solo hacia localhost)");
      }
      cloud = {
        ...cloud,
        mode: "hybrid",
        paired: true,
        baseUrl: request.url,
        deviceId: "0192f0a0-0000-7000-8000-00000000d0d0",
        deviceName: request.deviceName,
        online: true,
        latencyMs: 48,
        vectorSearch: "pgvector",
        lastSyncAt: new Date().toISOString(),
        syncedMemories: memories.length - 1,
        pending: 1,
      };
      conflicts = [
        {
          id: "conf-demo",
          entity: "memory",
          entityId: memories[0]?.id ?? "m",
          kind: "update_update",
          fields: ["content"],
          serverVersion: 3,
          serverData: { content: "Reunión del equipo los miércoles a las 9 (demostración)" },
          deviceData: { content: "Reunión del equipo los miércoles a las 10 (demostración)" },
          createdAt: new Date().toISOString(),
        },
      ];
      return { ...cloud, conflicts: conflicts.length };
    },
    async cloudUnpair() {
      cloud = { ...cloud, mode: "local", paired: false, baseUrl: null, deviceId: null, deviceName: null, online: false, latencyMs: null, lastSyncAt: null, pending: 0, syncedMemories: 0 };
      conflicts = [];
      return { ...cloud, conflicts: 0 };
    },
    async cloudSetMode(mode) {
      cloud = { ...cloud, mode };
      return { ...cloud, conflicts: conflicts.length };
    },
    async cloudSyncNow() {
      await sleep(delayMs * 2);
      if (!cloud.paired) throw new Error("este equipo no está emparejado");
      cloud = { ...cloud, pending: 0, syncedMemories: memories.length, lastSyncAt: new Date().toISOString() };
      return { pushed: 1, merged: 0, conflicts: 0, rejected: 0, pulled: 3, memoriesChanged: 3, memoriesDeleted: 0 };
    },
    cloudConflicts: async () => conflicts,
    async cloudResolveConflict(conflictId) {
      conflicts = conflicts.filter((c) => c.id !== conflictId);
      return { ...cloud, conflicts: conflicts.length };
    },
    getInference: async () => inference,
    async updateInference(settings, apiKey) {
      if (settings.mode !== "local" && !settings.remoteUrl.trim()) throw new Error("falta la URL del proveedor remoto");
      const remote = settings.remoteUrl.trim() !== "";
      inference = {
        ...inference,
        settings,
        hasApiKey: apiKey === "" ? false : apiKey ? true : inference.hasApiKey,
        status: { ...inference.status, mode: settings.mode, remoteConfigured: remote, remoteName: remote ? `${settings.remoteKind} (${settings.remoteModel})` : null },
      };
      return inference;
    },
    testInference: async () => ({ ok: false, detail: "vista previa: no hay Ollama ni proveedor remoto conectados" }),
    async onVisualEvent(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
  return backend;
}
