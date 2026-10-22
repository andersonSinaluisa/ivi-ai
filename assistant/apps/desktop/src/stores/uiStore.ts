import { create } from "zustand";

export type Screen = "dashboard" | "chat" | "tasks" | "memory" | "skills" | "settings" | "activity";

interface UiState {
  screen: Screen;
  /** Recuerdo que la pantalla de Memoria debe abrir. */
  memoryFocus: string | null;
  go(screen: Screen): void;
  openMemory(id: string | null): void;
}

export const useUi = create<UiState>((set) => ({
  screen: "dashboard",
  memoryFocus: null,
  go: (screen) => set({ screen }),
  openMemory: (id) => set({ memoryFocus: id }),
}));
