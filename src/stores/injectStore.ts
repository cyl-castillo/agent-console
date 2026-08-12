import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { DocFeedback, InjectionRecord } from "../types/domain";

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
  /// Outcome stats per corpus doc id (E2): usage counts + your verdicts.
  feedback: Record<string, DocFeedback>;

  load: (projectRoot: string) => Promise<void>;
  setEnabled: (projectRoot: string, enabled: boolean) => Promise<void>;
  /// One 👍/👎 on a doc. Verdicts move the injection ranking only — bounded
  /// nudge plus the hard exclusion the backend enforces (3× 👎, no 👍).
  vote: (projectRoot: string, docId: string, helpful: boolean) => Promise<void>;
  /// Rehabilitate an excluded doc (wipes verdicts, keeps usage history).
  resetVerdicts: (projectRoot: string, docId: string) => Promise<void>;
  _onInjected: (r: InjectionRecord) => void;
}

export const useInjectStore = create<InjectState>((set, get) => ({
  recent: [],
  enabled: true,
  lastByTerm: {},
  feedback: {},

  load: async (projectRoot) => {
    try {
      const [enabled, recent, stats] = await Promise.all([
        ipc.memoryInjectionEnabled(projectRoot),
        ipc.memoryInjectionRecent(),
        ipc.memoryFeedbackStats(projectRoot),
      ]);
      set({ enabled, recent, lastByTerm: byTerm(recent), feedback: byDoc(stats) });
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

  vote: async (projectRoot, docId, helpful) => {
    try {
      const updated = await ipc.memoryFeedbackSet(projectRoot, docId, helpful);
      set((s) => ({ feedback: { ...s.feedback, [docId]: updated } }));
    } catch {
      /* verdicts are advisory — a failed click just doesn't count */
    }
  },

  resetVerdicts: async (projectRoot, docId) => {
    try {
      const updated = await ipc.memoryFeedbackReset(projectRoot, docId);
      set((s) => ({ feedback: { ...s.feedback, [docId]: updated } }));
    } catch {
      /* same as vote */
    }
  },

  _onInjected: (r) => {
    set((s) => {
      // Mirror the backend's passive usage bump so counters stay live
      // without a refetch.
      const feedback = { ...s.feedback };
      for (const h of r.hits) {
        const prev = feedback[h.id] ?? {
          docId: h.id,
          injectedCount: 0,
          helpful: 0,
          unhelpful: 0,
          lastInjectedMs: 0,
          excluded: false,
        };
        feedback[h.id] = { ...prev, injectedCount: prev.injectedCount + 1, lastInjectedMs: r.tsMs };
      }
      return {
        recent: [r, ...s.recent].slice(0, 20),
        lastByTerm: r.termId ? { ...s.lastByTerm, [r.termId]: r } : s.lastByTerm,
        feedback,
      };
    });
  },
}));

function byDoc(stats: DocFeedback[]): Record<string, DocFeedback> {
  const out: Record<string, DocFeedback> = {};
  for (const d of stats) out[d.docId] = d;
  return out;
}

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
