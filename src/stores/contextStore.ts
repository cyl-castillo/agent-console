import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { ContextStatus, MemoryEntry, SemanticHit } from "../types/domain";
import { useSessionStore } from "./sessionStore";
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

  /// Semantic (embedding) search over memories + skills. First use downloads
  /// the local model (~100MB) — `searching` covers that too.
  searchHits: SemanticHit[];
  searching: boolean;
  searchError: string | null;
  semanticSearch: (query: string) => Promise<void>;
  clearSearch: () => void;
  reindexing: boolean;
  semanticReindex: () => Promise<string | null>;
}

export const useContextStore = create<ContextState>((set, get) => ({
  status: null,
  memories: [],
  loading: false,
  error: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const [status, memories] = await Promise.all([ipc.contextStatus(), ipc.memoryList()]);
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

  searchHits: [],
  searching: false,
  searchError: null,
  semanticSearch: async (query) => {
    const root = useSessionStore.getState().project?.root;
    const q = query.trim();
    if (!root || !q) return;
    set({ searching: true, searchError: null });
    try {
      const hits = await ipc.semanticSearch(root, q, 8);
      set({ searchHits: hits, searching: false });
    } catch (e) {
      set({ searching: false, searchError: String(e) });
    }
  },
  clearSearch: () => set({ searchHits: [], searchError: null }),

  reindexing: false,
  semanticReindex: async () => {
    const root = useSessionStore.getState().project?.root;
    if (!root) return null;
    set({ reindexing: true, searchError: null });
    try {
      const r = await ipc.semanticReindex(root);
      set({ reindexing: false });
      return `${r.total} indexed (${r.indexed} new, ${r.reused} unchanged)`;
    } catch (e) {
      set({ reindexing: false, searchError: String(e) });
      return null;
    }
  },
}));
