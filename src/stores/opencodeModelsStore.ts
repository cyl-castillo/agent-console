import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { OpencodeModel } from "../types/domain";

/// OpenCode's model catalog for the open project, read live from the CLI
/// (`opencode api GET /api/model`). Unlike Claude's aliases or Codex's effort
/// levels there is no stable list to ship: the ids are `provider/model` for
/// whatever providers the user configured, so the pickers ask here.
interface OpencodeModelsState {
  models: OpencodeModel[];
  loadedFor: string | null;
  loading: boolean;
  error: string | null;
  /// Fetch (once per project root; `force` refetches).
  load: (root: string, force?: boolean) => Promise<void>;
}

export const useOpencodeModelsStore = create<OpencodeModelsState>((set, get) => ({
  models: [],
  loadedFor: null,
  loading: false,
  error: null,

  load: async (root, force = false) => {
    const { loadedFor, loading } = get();
    if (loading || (!force && loadedFor === root)) return;
    set({ loading: true, error: null });
    try {
      const models = await ipc.opencodeModels();
      set({ models, loadedFor: root, error: null });
    } catch (err) {
      // No OpenCode on PATH, or a folder it has not served yet: the picker
      // falls back to the free-text model entry.
      set({ models: [], loadedFor: root, error: err instanceof Error ? err.message : String(err) });
    } finally {
      set({ loading: false });
    }
  },
}));
