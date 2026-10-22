// Acceso al backend Rust. En Tauri usa `invoke`, `Channel` y eventos; fuera de Tauri (vite en el
// navegador o pruebas) usa `mockBackend.ts`, que se indica en pantalla como vista previa.
import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AgentEvent,
  AuditEntry,
  BackendVisualEvent,
  BackupInfo,
  ConsolidationReport,
  Conversation,
  Experience,
  ExportOptions,
  ExportReport,
  GraphView,
  InferenceSettings,
  InferenceTest,
  InferenceView,
  LocalConflict,
  PairRequest,
  ProbeResult,
  SyncMode,
  SyncReport,
  SyncStatus,
  ImportPreview,
  ImportReport,
  ImportSelection,
  MemoryDetail,
  MemoryFilter,
  MemoryKind,
  MemoryPage,
  MemoryPatch,
  MemoryRecord,
  MemoryRelation,
  MemoryStats,
  RelationKind,
  RememberOutcome,
  RetrievalResult,
  RunInfo,
  RunSummary,
  Settings,
  Skill,
  SkillVersion,
  StoredMessage,
  SystemStatus,
  ToolInfo,
  ToolPolicyMode,
  Verification,
  VoiceConnection,
} from "@jarvis/shared-types";
import { createMockBackend } from "./mockBackend";

export { createMockBackend };

export interface AgentLinkStatus {
  ok: boolean;
  detail: string;
  toolCount: number;
}

export interface NewMemoryInput {
  content: string;
  summary?: string;
  kind?: MemoryKind;
  tags?: string[];
}

export interface GraphQuery {
  focus?: string;
  depth?: number;
  limit?: number;
  offset?: number;
}

export interface Backend {
  readonly kind: "tauri" | "mock";
  sendMessage(conversationId: string | null, text: string, onEvent: (e: AgentEvent) => void): Promise<RunInfo>;
  cancelRun(runId: string): Promise<boolean>;
  resolveApproval(approvalId: string, approved: boolean): Promise<boolean>;
  listConversations(): Promise<Conversation[]>;
  getMessages(conversationId: string): Promise<StoredMessage[]>;
  deleteConversation(conversationId: string): Promise<boolean>;
  getStatus(): Promise<SystemStatus>;
  listModels(): Promise<string[]>;
  listAudit(limit?: number): Promise<AuditEntry[]>;
  listRuns(limit?: number): Promise<RunSummary[]>;
  listTools(): Promise<ToolInfo[]>;
  setToolMode(tool: string, mode: ToolPolicyMode): Promise<void>;
  refreshWindowsAgent(): Promise<AgentLinkStatus>;
  getSettings(): Promise<Settings>;
  updateSettings(settings: Settings): Promise<Settings>;
  voiceConnection(): Promise<VoiceConnection>;

  // Memoria
  listMemories(filter: MemoryFilter, offset: number, limit: number): Promise<MemoryPage>;
  getMemory(id: string): Promise<MemoryDetail>;
  addMemory(input: NewMemoryInput): Promise<RememberOutcome>;
  updateMemory(id: string, patch: MemoryPatch): Promise<MemoryRecord>;
  setMemoryVerification(id: string, verification: Verification): Promise<MemoryRecord>;
  deleteMemory(id: string): Promise<void>;
  addRelation(sourceId: string, targetId: string, kind: RelationKind, note?: string): Promise<MemoryRelation>;
  verifyRelation(id: string, verified: boolean): Promise<MemoryRelation>;
  deleteRelation(id: string): Promise<void>;
  memoryGraph(query: GraphQuery): Promise<GraphView>;
  searchMemories(query: string): Promise<RetrievalResult>;
  memoryStats(): Promise<MemoryStats>;
  consolidateMemory(): Promise<ConsolidationReport>;
  refreshEmbeddings(): Promise<string>;
  listExperiences(): Promise<Experience[]>;
  promoteExperience(id: string): Promise<Skill>;
  listSkills(): Promise<Skill[]>;
  skillVersions(id: string): Promise<SkillVersion[]>;
  setSkillEnabled(id: string, enabled: boolean): Promise<Skill>;
  updateSkill(id: string, procedure: string, notes?: string): Promise<Skill>;
  defaultExportPath(): Promise<string>;
  listMemoryPackages(): Promise<string[]>;
  exportMemory(path: string | null, passphrase: string, options: ExportOptions): Promise<ExportReport>;
  previewImport(path: string, passphrase: string): Promise<ImportPreview>;
  applyImport(path: string, passphrase: string, selection: ImportSelection): Promise<ImportReport>;
  listBackups(): Promise<BackupInfo[]>;
  createBackup(): Promise<string>;
  restoreBackup(name: string): Promise<string>;

  // Cloud Core e Inference Gateway
  cloudStatus(): Promise<SyncStatus>;
  cloudProbe(url?: string): Promise<ProbeResult>;
  cloudPair(request: PairRequest): Promise<SyncStatus>;
  cloudUnpair(): Promise<SyncStatus>;
  cloudSetMode(mode: SyncMode): Promise<SyncStatus>;
  cloudSyncNow(): Promise<SyncReport>;
  cloudConflicts(): Promise<LocalConflict[]>;
  cloudResolveConflict(id: string, resolution: "keep_server" | "keep_device" | "custom", data?: Record<string, unknown>): Promise<SyncStatus>;
  getInference(): Promise<InferenceView>;
  updateInference(settings: InferenceSettings, apiKey?: string | null): Promise<InferenceView>;
  testInference(): Promise<InferenceTest>;

  /** Eventos visuales (Memory Engine y agente). Devuelve la función para dejar de escuchar. */
  onVisualEvent(cb: (e: BackendVisualEvent) => void): Promise<() => void>;
}

/** Los errores de los comandos llegan como string; se normalizan a Error. */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new Error(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
  }
}

export const tauriBackend: Backend = {
  kind: "tauri",
  sendMessage(conversationId, text, onEvent) {
    const channel = new Channel<AgentEvent>();
    channel.onmessage = onEvent;
    return call<RunInfo>("send_message", { conversationId, text, onEvent: channel });
  },
  cancelRun: (runId) => call("cancel_run", { runId }),
  resolveApproval: (approvalId, approved) => call("resolve_approval", { approvalId, approved }),
  listConversations: () => call("list_conversations"),
  getMessages: (conversationId) => call("get_messages", { conversationId }),
  deleteConversation: (conversationId) => call("delete_conversation", { conversationId }),
  getStatus: () => call("get_status"),
  listModels: () => call("list_models"),
  listAudit: (limit) => call("list_audit", { limit }),
  listRuns: (limit) => call("list_runs", { limit }),
  listTools: () => call("list_tools"),
  setToolMode: (tool, mode) => call("set_tool_mode", { tool, mode }),
  refreshWindowsAgent: () => call("refresh_windows_agent"),
  getSettings: () => call("get_settings"),
  updateSettings: (settings) => call("update_settings", { settings }),
  voiceConnection: () => call("voice_connection"),

  listMemories: (filter, offset, limit) => call("list_memories", { filter, offset, limit }),
  getMemory: (id) => call("get_memory", { id }),
  addMemory: (input) => call("add_memory", { input }),
  updateMemory: (id, patch) => call("update_memory", { id, patch }),
  setMemoryVerification: (id, verification) => call("set_memory_verification", { id, verification }),
  deleteMemory: (id) => call("delete_memory", { id }),
  addRelation: (sourceId, targetId, kind, note) => call("add_relation", { sourceId, targetId, kind, note }),
  verifyRelation: (id, verified) => call("verify_relation", { id, verified }),
  deleteRelation: (id) => call("delete_relation", { id }),
  memoryGraph: (q) => call("memory_graph", { focus: q.focus, depth: q.depth, limit: q.limit, offset: q.offset }),
  searchMemories: (query) => call("search_memories", { query }),
  memoryStats: () => call("memory_stats"),
  consolidateMemory: () => call("consolidate_memory"),
  refreshEmbeddings: () => call("refresh_embeddings"),
  listExperiences: () => call("list_experiences"),
  promoteExperience: (id) => call("promote_experience", { id }),
  listSkills: () => call("list_skills"),
  skillVersions: (id) => call("skill_versions", { id }),
  setSkillEnabled: (id, enabled) => call("set_skill_enabled", { id, enabled }),
  updateSkill: (id, procedure, notes) => call("update_skill", { id, procedure, notes }),
  defaultExportPath: () => call("default_export_path"),
  listMemoryPackages: () => call("list_memory_packages"),
  exportMemory: (path, passphrase, options) => call("export_memory", { path, passphrase, options }),
  previewImport: (path, passphrase) => call("preview_import", { path, passphrase }),
  applyImport: (path, passphrase, selection) => call("apply_import", { path, passphrase, selection }),
  listBackups: () => call("list_backups"),
  createBackup: () => call("create_backup"),
  restoreBackup: (name) => call("restore_backup", { name }),

  cloudStatus: () => call("cloud_status"),
  cloudProbe: (url) => call("cloud_probe", { url: url ?? null }),
  cloudPair: (request) => call("cloud_pair", { request }),
  cloudUnpair: () => call("cloud_unpair"),
  cloudSetMode: (mode) => call("cloud_set_mode", { mode }),
  cloudSyncNow: () => call("cloud_sync_now"),
  cloudConflicts: () => call("cloud_conflicts"),
  cloudResolveConflict: (id, resolution, data) => call("cloud_resolve_conflict", { id, resolution, data: data ?? null }),
  getInference: () => call("get_inference"),
  updateInference: (settings, apiKey) => call("update_inference", { settings, apiKey: apiKey ?? null }),
  testInference: () => call("test_inference"),

  async onVisualEvent(cb) {
    return listen<BackendVisualEvent>("jarvis://visual", (e) => cb(e.payload));
  },
};

let current: Backend | null = null;

export function backend(): Backend {
  if (!current) {
    current = isTauri() ? tauriBackend : createMockBackend();
  }
  return current;
}

/** Para pruebas. */
export function setBackend(b: Backend): void {
  current = b;
}
