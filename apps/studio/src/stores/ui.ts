// UI store: selection and tabs only — presentation state, never DRM logic
// (ui-arch §Frontend State). Workspace semantics live in the engine.
import { create } from "zustand";

/** Presentation state owned by the renderer. */
export interface UiState {
  selection: string | null;
  tabs: string[];
  select(id: string | null): void;
  openTab(id: string): void;
  closeTab(id: string): void;
}

/** Zustand store for selection/tabs (no workspace semantics). */
export const useUiStore = create<UiState>((set) => ({
  selection: null,
  tabs: [],
  select: (selection) => set({ selection }),
  openTab: (id) =>
    set((state) => ({
      tabs: state.tabs.includes(id) ? state.tabs : [...state.tabs, id],
    })),
  closeTab: (id) =>
    set((state) => ({
      tabs: state.tabs.filter((tab) => tab !== id),
    })),
}));
