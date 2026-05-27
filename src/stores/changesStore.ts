import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
  stageMany: (files: string[]) => Promise<void>;
  unstageMany: (files: string[]) => Promise<void>;
  revert: (file: string) => Promise<void>;
  revertAll: () => Promise<void>;
  commit: (opts?: { amend?: boolean }) => Promise<string | null>;
  loadCommitHistory: () => Promise<void>;
  loadHeadMessage: () => Promise<string>;
  recentMessages: string[];
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
  recentMessages: [],

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

  stageMany: async (files) => {
    for (const f of files) {
      try { await ipc.gitStageFile(f); } catch { /* keep going */ }
    }
    await get().refresh();
  },

  unstageMany: async (files) => {
    for (const f of files) {
      try { await ipc.gitUnstageFile(f); } catch { /* keep going */ }
    }
    await get().refresh();
  },

  loadCommitHistory: async () => {
    try {
      const msgs = await ipc.gitRecentMessages(10);
      set({ recentMessages: msgs });
    } catch { /* ignore */ }
  },

  loadHeadMessage: async () => {
    try { return await ipc.gitHeadMessage(); }
    catch { return ""; }
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

  commit: async (opts) => {
    const msg = get().commitMessage.trim();
    if (!msg) return null;
    set({ committing: true, error: null });
    try {
      const sha = opts?.amend
        ? await ipc.gitAmendCommit(msg)
        : await ipc.gitCommit(msg);
      set({ commitMessage: "", committing: false });
      await get().refresh();
      await get().loadCommitHistory();
      return sha;
    } catch (e) {
      set({ error: String(e), committing: false });
      return null;
    }
  },

  clear: () => set({
    status: null, selected: null, diff: "", error: null,
    commitMessage: "", committing: false, recentMessages: [],
  }),
}));

/// Subscribe once to the backend `git://changed` filesystem watcher and
/// trigger a debounced refresh of the Changes view. Returns an unlisten fn
/// for cleanup. The debounce smooths bursts (e.g. a build that touches many
/// files in <500ms) so we don't thrash `git status`.
export async function attachGitWatcherListener(): Promise<UnlistenFn> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const debouncedRefresh = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      useChangesStore.getState().refresh();
    }, 300);
  };
  return await listen("git://changed", debouncedRefresh);
}
