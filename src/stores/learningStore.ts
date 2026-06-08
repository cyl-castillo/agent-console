import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { LearningSuggestion } from "../types/domain";
import { useSkillsStore } from "./skillsStore";
import { useContextStore } from "./contextStore";

export type LearningStatus = "idle" | "reflecting" | "results" | "error";

/// One suggestion as shown in the UI, with local-only state for whether the
/// user has already acted on it. "friction" suggestions are report-only — they
/// carry no apply action, only skip.
export interface LearningItem extends LearningSuggestion {
  id: string;
  status: "proposed" | "applying" | "applied" | "skipped" | "error";
  appliedPath?: string;
  errorMessage?: string;
}

interface LearningState {
  status: LearningStatus;
  items: LearningItem[];
  errorMessage: string | null;
  rawExcerpt: string | null;
  eventsAnalyzed: number;

  reflect: () => Promise<void>;
  apply: (id: string) => Promise<void>;
  skip: (id: string) => void;
  reset: () => void;
}

export const useLearningStore = create<LearningState>((set, get) => ({
  status: "idle",
  items: [],
  errorMessage: null,
  rawExcerpt: null,
  eventsAnalyzed: 0,

  reflect: async () => {
    set({ status: "reflecting", errorMessage: null, items: [], rawExcerpt: null });
    try {
      const result = await ipc.learningReflect();
      const items: LearningItem[] = result.suggestions.map((s, i) => ({
        ...s,
        id: `${Date.now()}-${i}`,
        status: "proposed",
      }));
      set({
        status: "results",
        items,
        rawExcerpt: result.rawExcerpt,
        eventsAnalyzed: result.eventsAnalyzed,
      });
    } catch (err) {
      set({
        status: "error",
        errorMessage: err instanceof Error ? err.message : String(err),
      });
    }
  },

  apply: async (id) => {
    const item = get().items.find((it) => it.id === id);
    if (!item) return;
    // Only skill/memory suggestions can be materialized; friction is report-only.
    if (item.kind === "friction") return;

    set((s) => ({
      items: s.items.map((it) =>
        it.id === id ? { ...it, status: "applying", errorMessage: undefined } : it,
      ),
    }));
    try {
      let path: string;
      if (item.kind === "skill") {
        if (!item.skillName || !item.skillMdContent) {
          throw new Error("suggestion is missing skill content");
        }
        path = await ipc.learningCreateSkill(item.skillName, item.skillMdContent);
        useSkillsStore.getState().refresh();
      } else {
        if (!item.memoryName || !item.memoryContent) {
          throw new Error("suggestion is missing memory content");
        }
        path = await ipc.learningSaveMemory(item.memoryName, item.memoryContent);
        useContextStore.getState().refresh();
      }
      set((s) => ({
        items: s.items.map((it) =>
          it.id === id ? { ...it, status: "applied", appliedPath: path } : it,
        ),
      }));
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

  reset: () =>
    set({ status: "idle", items: [], errorMessage: null, rawExcerpt: null, eventsAnalyzed: 0 }),
}));
