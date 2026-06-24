import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { ContextStatus, MemoryEntry } from "../types/domain";
import { useLearningStore } from "./learningStore";
import { fireSchedulerEvent } from "./schedulerStore";

type Scope = "project" | "global";

/// Tracks the non-index memory count across refreshes so "corpus_grew" fires
/// only when a memory is actually added. -1 = no baseline yet.
let lastMemoryCount = -1;

interface ContextState {
  status: ContextStatus | null;
  memories: MemoryEntry[];
  loading: boolean;
  error: string | null;

  refresh: () => Promise<void>;
  readMd: (scope: Scope) => Promise<string>;
  writeMd: (scope: Scope, content: string, expectedMtimeMs: number | null) => Promise<void>;
  openExternally: (scope: Scope) => Promise<void>;
  generateStarter: () => Promise<string>;
  readMemory: (name: string) => Promise<string>;
  deleteMemory: (name: string) => Promise<void>;
}

export const useContextStore = create<ContextState>((set, get) => ({
  status: null,
  memories: [],
  loading: false,
  error: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const [status, memories] = await Promise.all([
        ipc.contextStatus(),
        ipc.memoryList(),
      ]);
      set({ status, memories, loading: false });
      // Corpus changed → maybe the curator should tidy it (threshold auto-trigger).
      useLearningStore.getState().noteCorpusSize();
      // Notify scheduler jobs watching for new memories (only on real growth).
      const n = memories.filter((m) => !m.isIndex).length;
      if (lastMemoryCount >= 0 && n > lastMemoryCount) void fireSchedulerEvent("corpus_grew");
      lastMemoryCount = n;
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  readMd: (scope) => ipc.contextReadMd(scope),

  writeMd: async (scope, content, expectedMtimeMs) => {
    await ipc.contextWriteMd(scope, content, expectedMtimeMs);
    await get().refresh();
  },

  openExternally: (scope) => ipc.contextOpenMdExternally(scope),

  generateStarter: () => ipc.contextGenerateStarter(),

  readMemory: (name) => ipc.memoryRead(name),

  deleteMemory: async (name) => {
    await ipc.memoryDelete(name);
    await get().refresh();
  },
}));
