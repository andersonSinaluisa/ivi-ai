// Cloud Core (sincronización) e Inference Gateway. Espejo de src-tauri/src/sync y commands/cloud.rs.

export type SyncMode = "local" | "hybrid";

export interface SyncStatus {
  mode: SyncMode;
  paired: boolean;
  baseUrl: string | null;
  deviceId: string | null;
  deviceName: string | null;
  /** Dónde se guarda la credencial del device (Credential Manager en Windows). */
  vault: string;
  online: boolean;
  latencyMs: number | null;
  vectorSearch: string | null;
  lastSyncAt: string | null;
  lastError: string | null;
  /** Registros con cambios locales pendientes de enviar. */
  pending: number;
  /** Cambios que el Cloud rechazó (no se reintentan). */
  rejected: number;
  conflicts: number;
  cursor: number;
  syncing: boolean;
  localMemories: number;
  syncedMemories: number;
}

export interface SyncReport {
  pushed: number;
  merged: number;
  conflicts: number;
  rejected: number;
  pulled: number;
  memoriesChanged: number;
  memoriesDeleted: number;
}

export interface LocalConflict {
  id: string;
  entity: string;
  entityId: string;
  kind: string;
  fields: string[];
  serverVersion: number;
  serverData: Record<string, unknown> | null;
  deviceData: Record<string, unknown> | null;
  createdAt: string;
}

export interface PairRequest {
  url: string;
  email: string;
  password: string;
  deviceName: string;
}

export interface ProbeResult {
  latencyMs: number;
  vectorSearch: string;
}

/** "server": Ollama de tu servidor (por defecto). "custom": proveedor propio con URL y clave. */
export type InferenceMode = "server" | "custom";

export interface InferenceSettings {
  mode: InferenceMode;
  /** "ollama" (Ollama remoto) u "openai" (API compatible con chat completions). */
  remoteKind: "ollama" | "openai";
  remoteUrl: string;
  remoteModel: string;
}

export interface GatewayStatus {
  mode: InferenceMode;
  customConfigured: boolean;
  customName: string | null;
  lastProvider: string | null;
}

/** Credencial del equipo para los servicios del servidor (la UI se la entrega al servicio de voz local). */
export interface ServiceCredential {
  serverUrl: string;
  token: string;
}

export interface InferenceView {
  settings: InferenceSettings;
  hasApiKey: boolean;
  status: GatewayStatus;
  vault: string;
}

export interface InferenceTest {
  ok: boolean;
  detail: string;
}
