import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { GitStatus } from "../types/domain";

interface ChangesState {
  status: GitStatus | null;
  selected: string | null;
  diff: string;
  loading: boolean;
  error: string | null;
  commitMessage: string;
  committing: boolean;

  setSelected: (file: string | null) => Promise<void>;
  setCommitMessage: (msg: string) => void;
  refresh: () => Promise<void>;
  stage: (file: string) => Promise<void>;
  unstage: (file: string) => Promise<void>;
  revert: (file: string) => Promise<void>;
  revertAll: () => Promise<void>;
  commit: () => Promise<string | null>;
  clear: () => void;
}

export const useChangesStore = create<ChangesState>((set, get) => ({
  status: null,
  selected: null,
  diff: "",
  loading: false,
  error: null,
  commitMessage: "",
  committing: false,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const status = await ipc.gitStatus();
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
      if (get().selected === file) set({ diff });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  setCommitMessage: (msg) => set({ commitMessage: msg }),

  stage: async (file) => {
    try {
      await ipc.gitStageFile(file);
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  unstage: async (file) => {
    try {
      await ipc.gitUnstageFile(file);
      await get().refresh();
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

  commit: async () => {
    const msg = get().commitMessage.trim();
    if (!msg) return null;
    set({ committing: true, error: null });
    try {
      const sha = await ipc.gitCommit(msg);
      set({ commitMessage: "", committing: false });
      await get().refresh();
      return sha;
    } catch (e) {
      set({ error: String(e), committing: false });
      return null;
    }
  },

  clear: () => set({
    status: null, selected: null, diff: "", error: null,
    commitMessage: "", committing: false,
  }),
}));
