// UI store: selection and tabs only — presentation state, never DRM logic
// (ui-arch §Frontend State). Workspace semantics live in the engine.
import { create } from "zustand";

/** Graph presentation state: viewport + collapse, never `drm.toml`. */
export interface GraphViewport {
  /** Zoom level (UI state only). */
  zoom: number;
  /** Whether placed nodes are collapsed to a count per compute. */
  collapsed: boolean;
}

/** Presentation state owned by the renderer. */
export interface UiState {
  selection: string | null;
  tabs: string[];
  graph: GraphViewport;
  select(id: string | null): void;
  openTab(id: string): void;
  closeTab(id: string): void;
  setGraphZoom(zoom: number): void;
  toggleGraphCollapsed(): void;
}

/** Zustand store for selection/tabs/graph viewport (no workspace semantics). */
export const useUiStore = create<UiState>((set) => ({
  selection: null,
  tabs: [],
  graph: { zoom: 1, collapsed: true },
  select: (selection) => set({ selection }),
  openTab: (id) =>
    set((state) => ({
      tabs: state.tabs.includes(id) ? state.tabs : [...state.tabs, id],
    })),
  closeTab: (id) =>
    set((state) => ({
      tabs: state.tabs.filter((tab) => tab !== id),
    })),
  setGraphZoom: (zoom) =>
    set((state) => ({ graph: { ...state.graph, zoom } })),
  toggleGraphCollapsed: () =>
    set((state) => ({ graph: { ...state.graph, collapsed: !state.graph.collapsed } })),
}));
