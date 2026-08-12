import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { InjectionRecord } from "../types/domain";

/// Memory injection (E1 of the knowledge flywheel): the backend feeds the
/// prompt hooks relevant memories; this store keeps what was injected — the
/// GUI's promise is that nothing is ever fed to the agent silently — plus the
/// per-project on/off toggle.
interface InjectState {
  /// Newest-first, as served by the backend (bounded there).
  recent: InjectionRecord[];
  /// Toggle for the CURRENT project (the one `load` was last called with).
  enabled: boolean;
  /// Last injection per terminal session — what the status-bar chip shows.
  lastByTerm: Record<string, InjectionRecord>;

  load: (projectRoot: string) => Promise<void>;
  setEnabled: (projectRoot: string, enabled: boolean) => Promise<void>;
  _onInjected: (r: InjectionRecord) => void;
}

export const useInjectStore = create<InjectState>((set, get) => ({
  recent: [],
  enabled: true,
  lastByTerm: {},

  load: async (projectRoot) => {
    try {
      const [enabled, recent] = await Promise.all([
        ipc.memoryInjectionEnabled(projectRoot),
        ipc.memoryInjectionRecent(),
      ]);
      set({ enabled, recent, lastByTerm: byTerm(recent) });
    } catch {
      /* injection is an enhancement — a failed load never surfaces */
    }
  },

  setEnabled: async (projectRoot, enabled) => {
    // Optimistic: the toggle must feel instant; revert on failure.
    const prev = get().enabled;
    set({ enabled });
    try {
      await ipc.memoryInjectionSetEnabled(projectRoot, enabled);
    } catch {
      set({ enabled: prev });
    }
  },

  _onInjected: (r) => {
    set((s) => ({
      recent: [r, ...s.recent].slice(0, 20),
      lastByTerm: r.termId ? { ...s.lastByTerm, [r.termId]: r } : s.lastByTerm,
    }));
  },
}));

function byTerm(recent: InjectionRecord[]): Record<string, InjectionRecord> {
  const out: Record<string, InjectionRecord> = {};
  // recent is newest-first; keep the first (= latest) record per terminal.
  for (const r of recent) {
    if (r.termId && !(r.termId in out)) out[r.termId] = r;
  }
  return out;
}

/// Subscribe once to the backend's `inject://done` event. Returns the
/// unlisten fn for cleanup (same pattern as the git watcher listener).
export async function attachInjectListener(): Promise<UnlistenFn> {
  return await listen<InjectionRecord>("inject://done", (e) => {
    useInjectStore.getState()._onInjected(e.payload);
  });
}
