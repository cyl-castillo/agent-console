import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { AdvisorRecommendation } from "../types/domain";
import { useSkillsStore } from "./skillsStore";

export type AdvisorStatus = "idle" | "analyzing" | "results" | "error";

/// One recommendation as shown in the UI, with local-only state for
/// tracking whether the user has already created it.
export interface AdvisorItem extends AdvisorRecommendation {
  id: string;
  status: "proposed" | "creating" | "created" | "skipped" | "error";
  createdPath?: string;
  errorMessage?: string;
  /// User can flip scope before clicking Create.
  scopeOverride?: "project" | "user";
}

interface AdvisorState {
  status: AdvisorStatus;
  items: AdvisorItem[];
  errorMessage: string | null;
  rawExcerpt: string | null;

  analyze: () => Promise<void>;
  setScope: (id: string, scope: "project" | "user") => void;
  create: (id: string) => Promise<void>;
  skip: (id: string) => void;
  reset: () => void;
}

export const useAdvisorStore = create<AdvisorState>((set, get) => ({
  status: "idle",
  items: [],
  errorMessage: null,
  rawExcerpt: null,

  analyze: async () => {
    set({ status: "analyzing", errorMessage: null, items: [], rawExcerpt: null });
    try {
      const result = await ipc.advisorAnalyze();
      const items: AdvisorItem[] = result.recommendations.map((r, i) => ({
        ...r,
        id: `${Date.now()}-${i}`,
        status: "proposed",
      }));
      set({ status: "results", items, rawExcerpt: result.rawExcerpt });
    } catch (err) {
      set({
        status: "error",
        errorMessage: err instanceof Error ? err.message : String(err),
      });
    }
  },

  setScope: (id, scope) => {
    set((s) => ({
      items: s.items.map((it) => (it.id === id ? { ...it, scopeOverride: scope } : it)),
    }));
  },

  create: async (id) => {
    const item = get().items.find((it) => it.id === id);
    if (!item) return;
    const scope = item.scopeOverride ?? (item.scope as "project" | "user");
    set((s) => ({
      items: s.items.map((it) =>
        it.id === id ? { ...it, status: "creating", errorMessage: undefined } : it,
      ),
    }));
    try {
      const path = await ipc.advisorCreateSkill(scope, item.name, item.skillMdContent);
      set((s) => ({
        items: s.items.map((it) =>
          it.id === id ? { ...it, status: "created", createdPath: path } : it,
        ),
      }));
      // Refresh the Skills panel so the new entry shows up.
      useSkillsStore.getState().refresh();
    } catch (err) {
      set((s) => ({
        items: s.items.map((it) =>
          it.id === id
            ? {
                ...it,
                status: "error",
                errorMessage: err instanceof Error ? err.message : String(err),
              }
            : it,
        ),
      }));
    }
  },

  skip: (id) => {
    set((s) => ({
      items: s.items.map((it) => (it.id === id ? { ...it, status: "skipped" } : it)),
    }));
  },

  reset: () => set({ status: "idle", items: [], errorMessage: null, rawExcerpt: null }),
}));
