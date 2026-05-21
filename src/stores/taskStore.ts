import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { AgentMode, Task, WorkspaceContext } from "../types/domain";
import { useSessionStore } from "./sessionStore";

interface TaskState {
  /// Tasks for the current session (in order, newest last).
  tasks: Task[];
  currentTaskId: string | null;

  /// Composer state — what the user is preparing for the next send.
  mode: AgentMode;
  constraints: string[];
  draftConstraint: string;

  /// Persisted history (lazy-loaded).
  history: Task[];
  historyOpen: boolean;

  /// Workspace context (lazy-loaded on project open).
  workspace: WorkspaceContext | null;

  setMode: (m: AgentMode) => void;
  addConstraint: (text: string) => void;
  removeConstraint: (index: number) => void;
  clearConstraints: () => void;
  setDraftConstraint: (s: string) => void;

  beginTask: (prompt: string) => Task;
  attachSnapshot: (taskId: string, commitSha: string) => void;
  recordFileRead: (path: string) => void;
  recordFileModified: (path: string) => void;
  recordCommand: (cmd: string) => void;
  completeTask: (taskId: string, cost: number | null, error: string | null) => Promise<void>;
  markRestored: (taskId: string) => void;
  resetSession: () => void;

  loadHistory: () => Promise<void>;
  toggleHistory: () => void;
  deleteHistoryItem: (id: string) => Promise<void>;

  loadWorkspaceContext: () => Promise<void>;
  clearWorkspaceContext: () => void;
}

const NOOP_TASK = (): Task => ({
  id: crypto.randomUUID(),
  projectRoot: "",
  prompt: "",
  mode: "build",
  constraints: [],
  createdAtMs: Date.now(),
  filesRead: [],
  filesModified: [],
  commandsExecuted: [],
});

export const useTaskStore = create<TaskState>((set, get) => ({
  tasks: [],
  currentTaskId: null,

  mode: "build",
  constraints: [],
  draftConstraint: "",

  history: [],
  historyOpen: false,

  workspace: null,

  setMode: (m) => set({ mode: m }),
  addConstraint: (text) => {
    const t = text.trim();
    if (!t) return;
    set((s) => ({ constraints: [...s.constraints, t], draftConstraint: "" }));
  },
  removeConstraint: (index) => set((s) => ({
    constraints: s.constraints.filter((_, i) => i !== index),
  })),
  clearConstraints: () => set({ constraints: [] }),
  setDraftConstraint: (s) => set({ draftConstraint: s }),

  beginTask: (prompt) => {
    const project = useSessionStore.getState().project;
    const task: Task = {
      ...NOOP_TASK(),
      projectRoot: project?.root ?? "",
      prompt,
      mode: get().mode,
      constraints: [...get().constraints],
      status: "running",
    };
    set((s) => ({ tasks: [...s.tasks, task], currentTaskId: task.id }));
    return task;
  },

  attachSnapshot: (taskId, commitSha) => set((s) => ({
    tasks: s.tasks.map((t) =>
      t.id === taskId ? { ...t, snapshotCommitSha: commitSha } : t,
    ),
  })),

  recordFileRead: (path) => {
    const id = get().currentTaskId;
    if (!id) return;
    set((s) => ({
      tasks: s.tasks.map((t) => {
        if (t.id !== id || t.filesRead.includes(path)) return t;
        return { ...t, filesRead: [...t.filesRead, path] };
      }),
    }));
  },

  recordFileModified: (path) => {
    const id = get().currentTaskId;
    if (!id) return;
    set((s) => ({
      tasks: s.tasks.map((t) => {
        if (t.id !== id || t.filesModified.includes(path)) return t;
        return { ...t, filesModified: [...t.filesModified, path] };
      }),
    }));
  },

  recordCommand: (cmd) => {
    const id = get().currentTaskId;
    if (!id) return;
    set((s) => ({
      tasks: s.tasks.map((t) =>
        t.id === id ? { ...t, commandsExecuted: [...t.commandsExecuted, cmd] } : t,
      ),
    }));
  },

  completeTask: async (taskId, cost, error) => {
    set((s) => ({
      tasks: s.tasks.map((t) => t.id === taskId ? {
        ...t,
        status: error ? "failed" : "completed",
        completedAtMs: Date.now(),
        costUsd: cost,
      } : t),
      currentTaskId: null,
    }));
    const task = get().tasks.find((t) => t.id === taskId);
    if (task) {
      try { await ipc.taskSave(task); } catch { /* ignore */ }
    }
  },

  markRestored: (taskId) => set((s) => ({
    tasks: s.tasks.map((t) => t.id === taskId
      ? { ...t, snapshotCommitSha: t.snapshotCommitSha } // keep sha; restored flag lives on the user block
      : t),
  })),

  resetSession: () => set({ tasks: [], currentTaskId: null }),

  loadHistory: async () => {
    const project = useSessionStore.getState().project;
    try {
      const items = await ipc.taskList(project?.root ?? null);
      set({ history: items });
    } catch { /* ignore */ }
  },
  toggleHistory: () => {
    set((s) => ({ historyOpen: !s.historyOpen }));
    if (!get().historyOpen) get().loadHistory();
  },
  deleteHistoryItem: async (id) => {
    try {
      await ipc.taskDelete(id);
      await get().loadHistory();
    } catch { /* ignore */ }
  },

  loadWorkspaceContext: async () => {
    try {
      const ws = await ipc.workspaceContext();
      set({ workspace: ws });
    } catch { /* ignore */ }
  },
  clearWorkspaceContext: () => set({ workspace: null }),
}));
