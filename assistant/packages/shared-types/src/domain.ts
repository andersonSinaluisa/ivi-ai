// Tipos de los comandos Tauri (apps/desktop/src-tauri/src/commands). camelCase en JSON.
import type { RiskClass, VerificationStatus } from "./agent";
import type { GraphNode, MemoryKind, Provenance, RelationKind, Verification } from "./visual";

export interface ServiceCheck {
  ok: boolean;
  detail: string;
}

export interface SystemStatus {
  ollama: ServiceCheck & { version: string | null };
  model: ServiceCheck & { name: string; installed: boolean; alternatives: string[] };
  windowsAgent: ServiceCheck & { toolCount: number };
  database: ServiceCheck;
  localTools: number;
}

export interface Settings {
  model: string;
  temperature: number;
  numCtx: number;
  think: boolean;
  maxSteps: number;
  autoApprove: RiskClass[];
}

export type ToolSource = "local" | "windows_agent";
export type ToolPolicyMode = "default" | "allow" | "confirm" | "deny";

export interface ToolInfo {
  id: string;
  name: string;
  description: string;
  risk: RiskClass;
  permissions: string[];
  timeoutMs: number;
  cancellable: boolean;
  source: ToolSource;
  parameters: unknown;
  policyMode: ToolPolicyMode;
  effectiveDecision: "allow" | "confirm" | "deny";
}

export interface Conversation {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
}

export interface StoredMessage {
  id: string;
  conversationId: string;
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  toolName: string | null;
  createdAt: string;
}

export type RetentionPolicy = { kind: "permanent" } | { kind: "days"; days: number } | { kind: "session" };
export type SourceKind = "user" | "agent" | "conversation" | "tool" | "import" | "experience" | "consolidation" | "device";

export interface MemoryRecord {
  id: string;
  kind: MemoryKind;
  content: string;
  summary: string;
  createdAt: string;
  updatedAt: string;
  source: string;
  sourceId: string | null;
  relevance: number;
  confidence: number;
  verification: Verification;
  tags: string[];
  references: string[];
  retention: RetentionPolicy;
  contentHash: string;
}

export interface MemorySource {
  id: string;
  kind: SourceKind;
  reference: string | null;
  label: string;
  createdAt: string;
}

export interface MemoryRelation {
  id: string;
  sourceId: string;
  targetId: string;
  kind: RelationKind;
  confidence: number;
  provenance: Provenance;
  verified: boolean;
  note: string | null;
  createdAt: string;
}

export interface RelationView {
  relation: MemoryRelation;
  other: GraphNode;
  outgoing: boolean;
}

export interface MemoryEventRow {
  id: number;
  ts: string;
  kind: string;
  runId: string | null;
  memoryIds: string[];
  detail: string | null;
}

export interface MemoryDetail {
  memory: MemoryRecord;
  source: MemorySource | null;
  relations: RelationView[];
  events: MemoryEventRow[];
}

export interface MemoryPage {
  items: MemoryRecord[];
  total: number;
  offset: number;
  limit: number;
}

export interface MemoryFilter {
  kind?: MemoryKind;
  verification?: Verification;
  query?: string;
  tag?: string;
}

export interface MemoryPatch {
  content?: string;
  summary?: string;
  tags?: string[];
  confidence?: number;
  verification?: Verification;
  retention?: RetentionPolicy;
}

export interface RememberOutcome {
  memory: MemoryRecord;
  created: boolean;
  relations: MemoryRelation[];
  embeddingPending: boolean;
}

export interface RetrievedMemory {
  memory: MemoryRecord;
  score: number;
  similarity: number;
  via: { type: "direct" } | { type: "graph"; relationId: string; from: string };
}

export interface RetrievalResult {
  intent: { kind: "remember" | "recall" | "task"; keywords: string[] };
  items: RetrievedMemory[];
  context: string;
  tokensEstimate: number;
  relationIds: string[];
  embeddingModel: string;
  semantic: boolean;
  degraded: boolean;
}

export interface EmbeddingStatus {
  model: string;
  semantic: boolean;
  detail: string;
  pending: number;
}

export interface MemoryStats {
  total: number;
  byKind: Record<string, number>;
  relations: number;
  skills: number;
  embedding: EmbeddingStatus;
  exportsDir: string;
}

export interface ConsolidationReport {
  removedExpired: number;
  mergedDuplicates: number;
  prunedRelations: number;
}

export interface Skill {
  id: string;
  name: string;
  description: string;
  procedure: string;
  enabled: boolean;
  requiresReview: boolean;
  source: string;
  experienceId: string | null;
  currentVersion: number;
  createdAt: string;
  updatedAt: string;
}

export interface SkillVersion {
  skillId: string;
  version: number;
  procedure: string;
  notes: string | null;
  createdAt: string;
}

export interface ExportOptions {
  kinds?: MemoryKind[];
  includeExperiences?: boolean;
  includeSkills?: boolean;
}

export interface Manifest {
  format: string;
  formatVersion: number;
  schemaVersion: number;
  appVersion: string;
  exportedAt: string;
  installationId: string;
  embeddingModel: string;
  counts: { memories: number; relations: number; sources: number; embeddings: number; experiences: number; skills: number };
  skills: { id: string; name: string; version: number }[];
  checksums: Record<string, string>;
}

export interface ExportReport {
  path: string;
  bytes: number;
  manifest: Manifest;
}

export type ConflictStrategy = "keep_local" | "keep_imported" | "keep_both";

export interface ImportSelection {
  kinds?: MemoryKind[];
  memoryIds?: string[];
  includeRelations?: boolean;
  includeExperiences?: boolean;
  includeSkills?: boolean;
  conflict?: ConflictStrategy;
}

export interface ImportPreview {
  manifest: Manifest;
  sameInstallation: boolean;
  newMemories: number;
  duplicates: number;
  conflicts: { id: string; importedSummary: string; localSummary: string }[];
  needsReembed: boolean;
  memories: { id: string; kind: MemoryKind; summary: string; status: "new" | "duplicate" | "conflict" }[];
}

export interface ImportReport {
  imported: number;
  duplicatesSkipped: number;
  conflictsKeptLocal: number;
  conflictsOverwritten: number;
  conflictsDuplicated: number;
  relations: number;
  embeddingsCopied: number;
  reembedded: number;
  experiences: number;
  skillsPendingReview: number;
  backup: string;
}

export interface BackupInfo {
  name: string;
  bytes: number;
  modified: string;
}

export interface ExperienceStep {
  tool: string;
  arguments: unknown;
  ok: boolean;
  verification: VerificationStatus;
}

export interface Experience {
  id: string;
  goal: string;
  context: string;
  plan: string;
  steps: ExperienceStep[];
  result: string;
  evidence: unknown[];
  errors: string[];
  procedure: string | null;
  verified: boolean;
  promoted: boolean;
  createdAt: string;
  completed: boolean;
  occurrences: number;
}

export interface AuditEntry {
  id: number;
  ts: string;
  runId: string | null;
  actor: string;
  action: string;
  tool: string | null;
  risk: RiskClass | null;
  decision: string | null;
  outcome: string | null;
  detail: string | null;
}

export interface RunSummary {
  id: string;
  conversationId: string;
  goal: string;
  status: "running" | "completed" | "failed" | "cancelled";
  startedAt: string;
  finishedAt: string | null;
  error: string | null;
  toolCalls: number;
}
