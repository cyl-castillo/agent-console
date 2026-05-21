import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { GitStatus } from "../types/domain";

interface ChangesState {
  status: GitStatus | null;
  selected: string | null;
  diff: string;
  loading: boolean;
  error: string | null;

  setSelected: (file: string | null) => Promise<void>;
  refresh: () => Promise<void>;
  revert: (file: string) => Promise<void>;
  revertAll: () => Promise<void>;
  clear: () => void;
}

export const useChangesStore = create<ChangesState>((set, get) => ({
  status: null,
  selected: null,
  diff: "",
  loading: false,
  error: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const status = await ipc.gitStatus();
      // Keep current selection if it still exists; otherwise pick the first.
      const prev = get().selected;
      const stillThere = prev && status.changes.find((c) => c.path === prev);
      const next = stillThere ? prev : (status.changes[0]?.path ?? null);
      set({ status, selected: next, loading: false });
      if (next) {
        await get().setSelected(next);
      } else {
        set({ diff: "" });
      }
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  setSelected: async (file) => {
    set({ selected: file });
    if (!file) { set({ diff: "" }); return; }
    try {
      const diff = await ipc.gitDiffFile(file);
      // Only commit if still selected (avoid races on rapid clicks).
      if (get().selected === file) set({ diff });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  revert: async (file) => {
    try {
      await ipc.gitRevertFile(file);
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  revertAll: async () => {
    const status = get().status;
    if (!status) return;
    for (const change of status.changes) {
      try { await ipc.gitRevertFile(change.path); } catch { /* keep going */ }
    }
    await get().refresh();
  },

  clear: () => set({ status: null, selected: null, diff: "", error: null }),
}));
