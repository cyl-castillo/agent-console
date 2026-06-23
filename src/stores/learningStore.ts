import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { CurationSuggestion, LearningSuggestion } from "../types/domain";
import { useSkillsStore } from "./skillsStore";
import { useContextStore } from "./contextStore";
import { useToastStore } from "./toastStore";

export type LearningStatus = "idle" | "reflecting" | "results" | "error";
export type CurationStatus = "idle" | "curating" | "results" | "error";

/// Auto-reflect fires once this many new prompts have been observed since the
/// last reflection — a chunk of work worth learning from, not every keystroke.
const AUTO_THRESHOLD = 15;
/// Minimum gap between auto-reflections, so a busy session can't spawn `claude`
/// (and spend tokens) back-to-back.
const AUTO_COOLDOWN_MS = 10 * 60 * 1000;

const AUTO_PREF_KEY = "agent-console:learning-auto";

/// Curation auto-trigger fires when the corpus has both grown past a floor and
/// added a chunk of new entries since the last pass — a corpus worth re-tidying,
/// not every single new skill. Threshold-based, as the user chose, rather than
/// scheduled or activity-counted.
const CURATE_MIN_CORPUS = 8;
const CURATE_GROWTH = 5;
/// Curation is heavier and rarer than reflection — a longer floor between passes.
const CURATE_COOLDOWN_MS = 30 * 60 * 1000;

const CURATE_AUTO_PREF_KEY = "agent-console:learning-curate-auto";
const CURATE_BASELINE_KEY = "agent-console:learning-curate-baseline";

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

function loadCurateAutoPref(): boolean {
  try {
    return localStorage.getItem(CURATE_AUTO_PREF_KEY) !== "0";
  } catch {
    return true;
  }
}

function saveCurateAutoPref(on: boolean) {
  try {
    localStorage.setItem(CURATE_AUTO_PREF_KEY, on ? "1" : "0");
  } catch {
    /* ignore */
  }
}

/// Corpus size at the last curation, persisted so growth is measured across
/// restarts (not reset to 0 every launch, which would re-curate on each start).
function loadCurateBaseline(): number {
  try {
    const v = Number(localStorage.getItem(CURATE_BASELINE_KEY));
    return Number.isFinite(v) ? v : 0;
  } catch {
    return 0;
  }
}

function saveCurateBaseline(n: number) {
  try {
    localStorage.setItem(CURATE_BASELINE_KEY, String(n));
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

/// A curation suggestion as shown in the UI. "rerank" is report-only (no apply),
/// mirroring how "friction" works for activity suggestions.
export interface CurationItem extends CurationSuggestion {
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

  /// Corpus curation — tends the *existing* skills/memories rather than growing
  /// them. Independent of the activity-reflection state above.
  curationStatus: CurationStatus;
  curationItems: CurationItem[];
  curationError: string | null;
  skillsAnalyzed: number;
  memoriesAnalyzed: number;

  /// Auto-curation: fires a pass once the corpus grows past the threshold.
  curateAutoEnabled: boolean;
  /// Corpus size (project skills + memories) at the last curation pass.
  lastCuratedSize: number;
  /// Epoch ms of the last curation, for the cooldown.
  lastCurateMs: number;

  curate: () => Promise<void>;
  applyCuration: (id: string) => Promise<void>;
  skipCuration: (id: string) => void;
  resetCuration: () => void;

  /// Called when the corpus inventory changes (skills/memories refreshed); may
  /// trigger an automatic curation once the growth threshold is crossed.
  noteCorpusSize: () => void;
  setCurateAutoEnabled: (on: boolean) => void;

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

  curationStatus: "idle",
  curationItems: [],
  curationError: null,
  skillsAnalyzed: 0,
  memoriesAnalyzed: 0,
  curateAutoEnabled: loadCurateAutoPref(),
  lastCuratedSize: loadCurateBaseline(),
  lastCurateMs: 0,

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

  curate: async () => {
    // Stamp the cooldown up front so a concurrent auto-trigger can't double-fire
    // while this pass is in flight (the status guard also covers the sync path).
    set({
      curationStatus: "curating",
      curationError: null,
      curationItems: [],
      lastCurateMs: Date.now(),
    });
    try {
      const result = await ipc.learningCurate();
      const items: CurationItem[] = result.suggestions.map((s, i) => ({
        ...s,
        id: `${Date.now()}-c${i}`,
        status: "proposed",
      }));
      // Re-baseline growth tracking to the corpus we just analyzed, so the next
      // auto-pass waits for genuinely new entries.
      const analyzed = result.skillsAnalyzed + result.memoriesAnalyzed;
      saveCurateBaseline(analyzed);
      set({
        curationStatus: "results",
        curationItems: items,
        skillsAnalyzed: result.skillsAnalyzed,
        memoriesAnalyzed: result.memoriesAnalyzed,
        lastCuratedSize: analyzed,
      });
    } catch (err) {
      set({
        curationStatus: "error",
        curationError: err instanceof Error ? err.message : String(err),
      });
    }
  },

  applyCuration: async (id) => {
    const item = get().curationItems.find((it) => it.id === id);
    if (!item) return;
    // "rerank" is report-only — there's nothing to materialize.
    if (item.action === "rerank") return;

    const patch = (changes: Partial<CurationItem>) =>
      set((s) => ({
        curationItems: s.curationItems.map((it) =>
          it.id === id ? { ...it, ...changes } : it,
        ),
      }));

    patch({ status: "applying", errorMessage: undefined });
    try {
      let path: string;
      if (item.action === "merge") {
        if (!item.newName || !item.newContent) {
          throw new Error("merge suggestion is missing new name/content");
        }
        path = await ipc.learningApplyMerge(
          item.targetKind,
          item.targets,
          item.newName,
          item.newContent,
        );
      } else if (item.action === "refactor") {
        if (!item.targets[0] || !item.newContent) {
          throw new Error("refactor suggestion is missing target/content");
        }
        // A refactor may also rename (newName differs from the target).
        const target = item.newName ?? item.targets[0];
        path = await ipc.learningApplyRefactor(item.targetKind, target, item.newContent);
      } else {
        // archive
        if (!item.targets[0]) throw new Error("archive suggestion has no target");
        path = await ipc.learningApplyArchive(item.targetKind, item.targets[0]);
      }
      // Reflect the corpus change in the live skills/memory views.
      if (item.targetKind === "skill") useSkillsStore.getState().refresh();
      else useContextStore.getState().refresh();
      patch({ status: "applied", appliedPath: path });
    } catch (err) {
      patch({
        status: "error",
        errorMessage: err instanceof Error ? err.message : String(err),
      });
    }
  },

  skipCuration: (id) => {
    set((s) => ({
      curationItems: s.curationItems.map((it) =>
        it.id === id ? { ...it, status: "skipped" } : it,
      ),
    }));
  },

  resetCuration: () =>
    set({
      curationStatus: "idle",
      curationItems: [],
      curationError: null,
      skillsAnalyzed: 0,
      memoriesAnalyzed: 0,
    }),

  noteCorpusSize: () => {
    const s = get();
    if (!s.curateAutoEnabled) return;
    if (s.curationStatus === "curating") return;

    // Current corpus = this project's skills + its memories (excluding the
    // hand-curated MEMORY.md index) — exactly what the curator analyzes.
    const skills = useSkillsStore
      .getState()
      .installed.filter((sk) => sk.source === "project" && sk.kind === "skill").length;
    const memories = useContextStore
      .getState()
      .memories.filter((m) => !m.isIndex).length;
    const size = skills + memories;

    if (size < CURATE_MIN_CORPUS) return;
    if (size - s.lastCuratedSize < CURATE_GROWTH) return;
    if (Date.now() - s.lastCurateMs < CURATE_COOLDOWN_MS) return;
    // Don't replace a batch the user is still reviewing.
    const pending = s.curationItems.filter(
      (it) => it.status === "proposed" || it.status === "error",
    ).length;
    if (pending > 0) return;

    void get().curate();
  },

  setCurateAutoEnabled: (on) => {
    saveCurateAutoPref(on);
    set({ curateAutoEnabled: on });
  },
}));
