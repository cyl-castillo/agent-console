import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { LearningSuggestion } from "../types/domain";
import { useSkillsStore } from "./skillsStore";
import { useContextStore } from "./contextStore";
import { useToastStore } from "./toastStore";

export type LearningStatus = "idle" | "reflecting" | "results" | "error";

/// Auto-reflect fires once this many new prompts have been observed since the
/// last reflection — a chunk of work worth learning from, not every keystroke.
const AUTO_THRESHOLD = 15;
/// Minimum gap between auto-reflections, so a busy session can't spawn `claude`
/// (and spend tokens) back-to-back.
const AUTO_COOLDOWN_MS = 10 * 60 * 1000;

const AUTO_PREF_KEY = "agent-console:learning-auto";

function loadAutoPref(): boolean {
  try {
    // Default ON: Fase 2 is opt-in by enabling auto-trigger, but the user can
    // turn it off from the panel and that choice sticks.
    return localStorage.getItem(AUTO_PREF_KEY) !== "0";
  } catch {
    return true;
  }
}

function saveAutoPref(on: boolean) {
  try {
    localStorage.setItem(AUTO_PREF_KEY, on ? "1" : "0");
  } catch {
    /* ignore */
  }
}

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

  /// Auto-trigger: when enabled, accumulating activity reflects on its own.
  autoEnabled: boolean;
  /// Whether the current/last results came from an automatic reflection.
  lastWasAuto: boolean;
  /// Prompts observed since the last reflection (manual or auto).
  sinceReflection: number;
  /// Epoch ms of the last auto-reflection, for the cooldown.
  lastAutoMs: number;

  reflect: () => Promise<void>;
  apply: (id: string) => Promise<void>;
  skip: (id: string) => void;
  reset: () => void;

  /// Called for every observed user prompt (wired from the hook stream); may
  /// trigger an automatic reflection once the threshold is crossed.
  noteActivity: () => void;
  setAutoEnabled: (on: boolean) => void;
}

/// Shared reflection runner for both manual and automatic triggers. Resets the
/// activity counter and cooldown so the next auto-reflect starts fresh. Auto
/// runs surface quietly (a toast); manual runs drive the panel UI as before.
async function runReflection(
  set: (partial: Partial<LearningState>) => void,
  auto: boolean,
): Promise<void> {
  set({
    status: "reflecting",
    errorMessage: null,
    items: [],
    rawExcerpt: null,
    lastWasAuto: auto,
    sinceReflection: 0,
    ...(auto ? { lastAutoMs: Date.now() } : {}),
  });
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
    if (auto && items.length > 0) {
      const n = items.length;
      useToastStore.getState().show(
        `Learning: ${n} new suggestion${n === 1 ? "" : "s"} from recent activity`,
        "info",
      );
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    // A failed auto-reflection stays silent in the UI (no error panel hijack);
    // a manual one shows the error so the user who asked sees what happened.
    if (auto) {
      set({ status: "idle" });
    } else {
      set({ status: "error", errorMessage: message });
    }
  }
}

export const useLearningStore = create<LearningState>((set, get) => ({
  status: "idle",
  items: [],
  errorMessage: null,
  rawExcerpt: null,
  eventsAnalyzed: 0,
  autoEnabled: loadAutoPref(),
  lastWasAuto: false,
  sinceReflection: 0,
  lastAutoMs: 0,

  reflect: () => runReflection(set, false),

  noteActivity: () => {
    const s = get();
    if (!s.autoEnabled) return;
    const since = s.sinceReflection + 1;
    set({ sinceReflection: since });

    if (s.status === "reflecting") return;
    if (since < AUTO_THRESHOLD) return;
    if (Date.now() - s.lastAutoMs < AUTO_COOLDOWN_MS) return;
    // Don't clobber a batch the user is still reviewing — the badge already
    // nudges them. Re-reflect only once the previous suggestions are handled.
    const pending = s.items.filter(
      (it) => it.status === "proposed" || it.status === "error",
    ).length;
    if (pending > 0) return;

    void runReflection(set, true);
  },

  setAutoEnabled: (on) => {
    saveAutoPref(on);
    set({ autoEnabled: on });
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
    set({
      status: "idle",
      items: [],
      errorMessage: null,
      rawExcerpt: null,
      eventsAnalyzed: 0,
      sinceReflection: 0,
      lastWasAuto: false,
    }),
}));
