import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { BranchInfo, GitStatus, PushResult } from "../types/domain";
import { useToastStore } from "./toastStore";
import { fireSchedulerEvent } from "./schedulerStore";

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
  /// Push the current branch (sets the upstream on first push) and remember
  /// the PR link it came back with. Null on failure (error toast shown).
  push: () => Promise<PushResult | null>;
  /// Open the PR/MR page for the current branch in the browser (pushing
  /// first is the user's call — the button says so when there is nothing to
  /// open yet).
  openPr: () => Promise<void>;
  pushing: boolean;
  /// PR link of the last push in this session, for the "Open PR" button.
  lastPrUrl: string | null;
  loadCommitHistory: () => Promise<void>;
  loadHeadMessage: () => Promise<string>;
  recentMessages: string[];

  branches: BranchInfo[];
  branchesLoading: boolean;
  loadBranches: () => Promise<void>;
  checkoutBranch: (name: string) => Promise<void>;
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
  branches: [],
  branchesLoading: false,
  pushing: false,
  lastPrUrl: null,

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
    if (!file) {
      set({ diff: "" });
      return;
    }
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
      try {
        await ipc.gitStageFile(f);
      } catch {
        /* keep going */
      }
    }
    await get().refresh();
  },

  unstageMany: async (files) => {
    for (const f of files) {
      try {
        await ipc.gitUnstageFile(f);
      } catch {
        /* keep going */
      }
    }
    await get().refresh();
  },

  loadCommitHistory: async () => {
    try {
      const msgs = await ipc.gitRecentMessages(10);
      set({ recentMessages: msgs });
    } catch {
      /* ignore */
    }
  },

  loadHeadMessage: async () => {
    try {
      return await ipc.gitHeadMessage();
    } catch {
      return "";
    }
  },

  loadBranches: async () => {
    set({ branchesLoading: true });
    try {
      const branches = await ipc.gitBranches();
      set({ branches, branchesLoading: false });
    } catch (e) {
      set({ error: String(e), branchesLoading: false });
    }
  },

  checkoutBranch: async (name) => {
    try {
      await ipc.gitCheckoutBranch(name);
      await get().refresh();
      await get().loadBranches();
    } catch (e) {
      set({ error: String(e) });
      throw e;
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
      try {
        await ipc.gitRevertFile(change.path);
      } catch {
        /* keep going */
      }
    }
    await get().refresh();
  },

  commit: async (opts) => {
    const msg = get().commitMessage.trim();
    if (!msg) return null;
    set({ committing: true, error: null });
    try {
      const sha = opts?.amend ? await ipc.gitAmendCommit(msg) : await ipc.gitCommit(msg);
      set({ commitMessage: "", committing: false });
      await get().refresh();
      await get().loadCommitHistory();
      // Notify scheduler jobs watching for commits (e.g. "review what I committed").
      void fireSchedulerEvent("commit");
      return sha;
    } catch (e) {
      set({ error: String(e), committing: false });
      return null;
    }
  },

  push: async () => {
    if (get().pushing) return null;
    set({ pushing: true, error: null });
    try {
      const r = await ipc.gitPush();
      set({ pushing: false, lastPrUrl: r.prUrl ?? null });
      await get().loadBranches();
      useToastStore
        .getState()
        .show(
          `Pushed ${r.branch} → ${r.remote}${r.setUpstream ? " (upstream set)" : ""}`,
          "success",
        );
      return r;
    } catch (e) {
      set({ pushing: false });
      useToastStore.getState().show(`Push failed: ${String(e).slice(0, 200)}`, "error");
      return null;
    }
  },

  openPr: async () => {
    let url = get().lastPrUrl;
    if (!url) {
      try {
        url = await ipc.gitPrUrl();
      } catch (e) {
        useToastStore.getState().show(`Couldn't resolve the PR link: ${e}`, "error");
        return;
      }
    }
    if (!url) {
      useToastStore
        .getState()
        .show(
          "No PR link: push the branch first, or this host isn't GitHub/GitLab — open the PR from your git host.",
          "info",
        );
      return;
    }
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch (e) {
      useToastStore.getState().show(`Couldn't open the browser: ${e}`, "error");
    }
  },

  clear: () =>
    set({
      status: null,
      selected: null,
      diff: "",
      error: null,
      commitMessage: "",
      committing: false,
      recentMessages: [],
      branches: [],
      branchesLoading: false,
      pushing: false,
      lastPrUrl: null,
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
