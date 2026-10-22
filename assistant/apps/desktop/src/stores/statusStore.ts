import { create } from "zustand";
import type { Settings, SystemStatus } from "@jarvis/shared-types";
import { backend } from "../services/backend";

interface StatusState {
  status: SystemStatus | null;
  settings: Settings | null;
  error: string | null;
  refresh(): Promise<void>;
  loadSettings(): Promise<void>;
  saveSettings(s: Settings): Promise<void>;
}

export const useStatus = create<StatusState>((set) => ({
  status: null,
  settings: null,
  error: null,
  async refresh() {
    try {
      set({ status: await backend().getStatus(), error: null });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },
  async loadSettings() {
    set({ settings: await backend().getSettings() });
  },
  async saveSettings(s) {
    set({ settings: await backend().updateSettings(s) });
  },
}));
