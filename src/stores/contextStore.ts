import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { ContextStatus, MemoryEntry } from "../types/domain";

type Scope = "project" | "global";

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
