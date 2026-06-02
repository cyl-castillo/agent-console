import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type {
  RoundtableActivity,
  RoundtableConfig,
  RoundtableStatus,
  RoundtableTurn,
} from "../types/domain";

export type RtPhase = "config" | "running" | "paused" | "done" | "stopped" | "error";

/// Draft of the debate setup, edited in the config form before launch.
export interface RtDraft {
  topic: string;
  nameA: string;
  modelA: string;
  personaA: string;
  nameB: string;
  modelB: string;
  personaB: string;
  maxRounds: number;
  tokenBudget: number;
  fullTools: boolean;
}

const DEFAULT_DRAFT: RtDraft = {
  topic: "",
  nameA: "Opus",
  modelA: "opus",
  personaA: "",
  nameB: "Sonnet",
  modelB: "sonnet",
  personaB: "",
  maxRounds: 4,
  tokenBudget: 400_000,
  fullTools: false,
};

interface RoundtableState {
  runId: string | null;
  phase: RtPhase;
  round: number;
  totalTokens: number;
  approxCostUsd: number;
  message: string | null;
  turns: RoundtableTurn[];
  /// Live activity lines (tool calls, reasoning, text) keyed by side+round.
  activities: RoundtableActivity[];

  injectDraft: string;
  /// Which side's full diff is open in the viewer (null = none).
  diffSide: string | null;
  diff: string;
  diffLoading: boolean;
  /// Snapshot sha taken before the last apply (for the "undo" hint).
  appliedSnapshot: string | null;
  appliedSide: string | null;

  draft: RtDraft;

  initListeners: () => Promise<void>;
  setDraft: (patch: Partial<RtDraft>) => void;
  start: () => Promise<void>;
  pause: () => Promise<void>;
  resume: () => Promise<void>;
  setInjectDraft: (v: string) => void;
  inject: () => Promise<void>;
  stop: () => Promise<void>;
  openDiff: (side: string) => Promise<void>;
  closeDiff: () => void;
  apply: (side: string) => Promise<void>;
  reset: () => Promise<void>;
}

let listenersBound = false;
let unlistenTurn: UnlistenFn | null = null;
let unlistenStatus: UnlistenFn | null = null;
let unlistenActivity: UnlistenFn | null = null;

export const useRoundtableStore = create<RoundtableState>((set, get) => ({
  runId: null,
  phase: "config",
  round: 0,
  totalTokens: 0,
  approxCostUsd: 0,
  message: null,
  turns: [],
  activities: [],

  injectDraft: "",
  diffSide: null,
  diff: "",
  diffLoading: false,
  appliedSnapshot: null,
  appliedSide: null,

  draft: { ...DEFAULT_DRAFT },

  initListeners: async () => {
    if (listenersBound) return;
    listenersBound = true;
    unlistenActivity = await listen<RoundtableActivity>("roundtable://activity", (e) => {
      const a = e.payload;
      if (a.id !== get().runId) return;
      // Coalesce consecutive "text" chunks from the same side+round so the
      // streamed answer reads as one growing block rather than many lines.
      set((s) => {
        const last = s.activities[s.activities.length - 1];
        if (
          a.kind === "text" &&
          last &&
          last.kind === "text" &&
          last.side === a.side &&
          last.round === a.round
        ) {
          const merged = { ...last, text: `${last.text}${a.text}` };
          return { activities: [...s.activities.slice(0, -1), merged] };
        }
        return { activities: [...s.activities, a] };
      });
    });
    unlistenTurn = await listen<RoundtableTurn>("roundtable://turn", (e) => {
      const t = e.payload;
      if (t.id !== get().runId) return;
      set((s) => ({
        turns: [...s.turns, t],
        totalTokens: t.totalTokens,
        round: t.round,
        approxCostUsd: s.approxCostUsd + (t.costUsd || 0),
      }));
    });
    unlistenStatus = await listen<RoundtableStatus>("roundtable://status", (e) => {
      const st = e.payload;
      if (st.id !== get().runId) return;
      const known: RtPhase[] = ["running", "paused", "done", "stopped", "error"];
      const phase = known.includes(st.status as RtPhase) ? (st.status as RtPhase) : get().phase;
      set({
        phase,
        round: st.round || get().round,
        totalTokens: st.totalTokens,
        message: st.message ?? get().message,
      });
    });
  },

  setDraft: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),

  start: async () => {
    const d = get().draft;
    if (!d.topic.trim()) {
      set({ message: "Describe the question to debate first." });
      return;
    }
    await get().initListeners();
    const config: RoundtableConfig = {
      topic: d.topic.trim(),
      participantA: { side: "a", name: d.nameA || "A", model: d.modelA, persona: d.personaA },
      participantB: { side: "b", name: d.nameB || "B", model: d.modelB, persona: d.personaB },
      maxRounds: Math.max(1, Math.min(20, d.maxRounds)),
      tokenBudget: Math.max(0, d.tokenBudget),
      fullTools: d.fullTools,
    };
    set({
      turns: [],
      activities: [],
      message: null,
      totalTokens: 0,
      approxCostUsd: 0,
      round: 0,
      appliedSnapshot: null,
      appliedSide: null,
      diffSide: null,
      diff: "",
      phase: "running",
    });
    try {
      const id = await ipc.roundtableStart(config);
      set({ runId: id });
    } catch (err) {
      set({ phase: "error", message: err instanceof Error ? err.message : String(err) });
    }
  },

  pause: async () => {
    const id = get().runId;
    if (!id) return;
    try {
      await ipc.roundtablePause(id);
      set({ phase: "paused" });
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  resume: async () => {
    const id = get().runId;
    if (!id) return;
    try {
      await ipc.roundtableResume(id);
      set({ phase: "running" });
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  setInjectDraft: (v) => set({ injectDraft: v }),

  inject: async () => {
    const id = get().runId;
    const msg = get().injectDraft.trim();
    if (!id || !msg) return;
    try {
      await ipc.roundtableInject(id, msg);
      set({ injectDraft: "" });
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  stop: async () => {
    const id = get().runId;
    if (!id) return;
    try {
      await ipc.roundtableStop(id);
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  openDiff: async (side) => {
    const id = get().runId;
    if (!id) return;
    set({ diffSide: side, diffLoading: true, diff: "" });
    try {
      const diff = await ipc.roundtableSideDiff(id, side);
      set({ diff, diffLoading: false });
    } catch (err) {
      set({ diffLoading: false, diff: "", message: err instanceof Error ? err.message : String(err) });
    }
  },

  closeDiff: () => set({ diffSide: null, diff: "" }),

  apply: async (side) => {
    const id = get().runId;
    if (!id) return;
    try {
      const snap = await ipc.roundtableApply(id, side);
      set({ appliedSnapshot: snap, appliedSide: side, message: null });
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  reset: async () => {
    const id = get().runId;
    if (id) {
      try {
        await ipc.roundtableDiscard(id);
      } catch {
        /* best effort */
      }
    }
    set({
      runId: null,
      phase: "config",
      round: 0,
      totalTokens: 0,
      approxCostUsd: 0,
      message: null,
      turns: [],
      activities: [],
      injectDraft: "",
      diffSide: null,
      diff: "",
      appliedSnapshot: null,
      appliedSide: null,
    });
  },
}));

export function teardownRoundtableListeners() {
  unlistenTurn?.();
  unlistenStatus?.();
  unlistenActivity?.();
  unlistenTurn = null;
  unlistenStatus = null;
  unlistenActivity = null;
  listenersBound = false;
}
