import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { profileFor } from "../agents/profiles";
import type {
  RoundtableActivity,
  RoundtableConfig,
  RoundtableParticipant,
  RoundtableStatus,
  RoundtableTurn,
} from "../types/domain";

export type RtPhase = "config" | "running" | "paused" | "done" | "stopped" | "error";

/// A roster entry edited in the config form before launch.
export interface RtParticipantDraft {
  id: string;
  name: string;
  engine: "claude" | "codex";
  model: string;
  role: string;
}

/// Draft of the room setup, edited in the config form before launch.
export interface RtDraft {
  problem: string;
  participants: RtParticipantDraft[];
  maxTurns: number;
  tokenBudget: number;
}

const DEFAULT_DRAFT: RtDraft = {
  problem: "",
  participants: [
    { id: "p1", name: "Opus", engine: "claude", model: "opus", role: "" },
    { id: "p2", name: "Codex", engine: "codex", model: "medium", role: "" },
  ],
  maxTurns: 6,
  tokenBudget: 400_000,
};

interface RoundtableState {
  runId: string | null;
  phase: RtPhase;
  turn: number;
  totalTokens: number;
  approxCostUsd: number;
  message: string | null;
  /// The shared conversation, in order (agent turns + human messages).
  turns: RoundtableTurn[];
  /// Live activity lines (reasoning, tool calls, streamed text) keyed by
  /// authorId+turn.
  activities: RoundtableActivity[];

  injectDraft: string;
  /// Wall-clock (ms) the current live AI turn began. Drives the per-turn clock.
  liveStartedAt: number | null;
  /// Wall-clock (ms) of the most recent activity event — the staleness signal.
  lastActivityAt: number | null;

  draft: RtDraft;
  /// The participants actually launched (ids reindexed p1..pN). Display uses
  /// this, not `draft`, so editing the roster mid-config never desyncs the
  /// running feed's names/colors from the backend's author ids.
  roster: RoundtableParticipant[];

  initListeners: () => Promise<void>;
  setDraft: (patch: Partial<RtDraft>) => void;
  addParticipant: () => void;
  removeParticipant: (id: string) => void;
  updateParticipant: (id: string, patch: Partial<RtParticipantDraft>) => void;
  start: () => Promise<void>;
  pause: () => Promise<void>;
  resume: () => Promise<void>;
  setInjectDraft: (v: string) => void;
  inject: () => Promise<void>;
  stop: () => Promise<void>;
  reset: () => Promise<void>;
}

let listenersBound = false;
let unlistenTurn: UnlistenFn | null = null;
let unlistenStatus: UnlistenFn | null = null;
let unlistenActivity: UnlistenFn | null = null;
let pidCounter = 2;

export const useRoundtableStore = create<RoundtableState>((set, get) => ({
  runId: null,
  phase: "config",
  turn: 0,
  totalTokens: 0,
  approxCostUsd: 0,
  message: null,
  turns: [],
  activities: [],

  injectDraft: "",
  liveStartedAt: null,
  lastActivityAt: null,

  draft: { ...DEFAULT_DRAFT, participants: DEFAULT_DRAFT.participants.map((p) => ({ ...p })) },
  roster: [],

  initListeners: async () => {
    if (listenersBound) return;
    listenersBound = true;
    unlistenActivity = await listen<RoundtableActivity>("roundtable://activity", (e) => {
      const a = e.payload;
      if (a.id !== get().runId) return;
      // Coalesce consecutive "text" chunks from the same author+turn so a
      // streamed answer reads as one growing block rather than many lines.
      set((s) => {
        const last = s.activities[s.activities.length - 1];
        if (
          a.kind === "text" &&
          last &&
          last.kind === "text" &&
          last.authorId === a.authorId &&
          last.turn === a.turn
        ) {
          const merged = { ...last, text: `${last.text}${a.text}` };
          return { activities: [...s.activities.slice(0, -1), merged], lastActivityAt: Date.now() };
        }
        return { activities: [...s.activities, a], lastActivityAt: Date.now() };
      });
    });
    unlistenTurn = await listen<RoundtableTurn>("roundtable://turn", (e) => {
      const t = e.payload;
      if (t.id !== get().runId) return;
      set((s) => {
        // Human injections don't advance token totals or the per-turn clock —
        // they just join the feed. (Backend sends totalTokens=0 for them.)
        if (t.isHuman) {
          return { turns: [...s.turns, t] };
        }
        return {
          turns: [...s.turns, t],
          totalTokens: t.totalTokens,
          turn: t.turn,
          approxCostUsd: s.approxCostUsd + (t.costUsd || 0),
          // A turn finished → the next begins now. Reset the per-turn clock.
          liveStartedAt: Date.now(),
          lastActivityAt: null,
        };
      });
    });
    unlistenStatus = await listen<RoundtableStatus>("roundtable://status", (e) => {
      const st = e.payload;
      if (st.id !== get().runId) return;
      const known: RtPhase[] = ["running", "paused", "done", "stopped", "error"];
      const phase = known.includes(st.status as RtPhase) ? (st.status as RtPhase) : get().phase;
      set({
        phase,
        turn: st.turn || get().turn,
        totalTokens: st.totalTokens || get().totalTokens,
        message: st.message ?? get().message,
      });
    });
  },

  setDraft: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),

  addParticipant: () =>
    set((s) => {
      pidCounter += 1;
      const id = `p${pidCounter}`;
      const next: RtParticipantDraft = { id, name: `Agent ${s.draft.participants.length + 1}`, engine: "claude", model: "sonnet", role: "" };
      return { draft: { ...s.draft, participants: [...s.draft.participants, next] } };
    }),

  removeParticipant: (id) =>
    set((s) => {
      // A room needs at least two voices.
      if (s.draft.participants.length <= 2) return s;
      return { draft: { ...s.draft, participants: s.draft.participants.filter((p) => p.id !== id) } };
    }),

  updateParticipant: (id, patch) =>
    set((s) => ({
      draft: {
        ...s.draft,
        participants: s.draft.participants.map((p) => (p.id === id ? { ...p, ...patch } : p)),
      },
    })),

  start: async () => {
    const d = get().draft;
    if (!d.problem.trim()) {
      set({ message: "Describe the problem the room should work on first." });
      return;
    }
    await get().initListeners();
    // Reindex ids p1.. so removals/reorders never leave gaps the backend keys on.
    const participants: RoundtableParticipant[] = d.participants.map((p, i) => ({
      id: `p${i + 1}`,
      name: p.name || `Agent ${i + 1}`,
      engine: p.engine,
      model: p.model,
      role: p.role,
    }));
    const config: RoundtableConfig = {
      problem: d.problem.trim(),
      participants,
      maxTurns: Math.max(1, Math.min(60, d.maxTurns)),
      tokenBudget: Math.max(0, d.tokenBudget),
    };
    set({
      turns: [],
      activities: [],
      message: null,
      totalTokens: 0,
      approxCostUsd: 0,
      turn: 0,
      roster: participants,
      phase: "running",
      liveStartedAt: Date.now(),
      lastActivityAt: null,
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
      turn: 0,
      totalTokens: 0,
      approxCostUsd: 0,
      message: null,
      turns: [],
      activities: [],
      injectDraft: "",
      liveStartedAt: null,
      lastActivityAt: null,
      roster: [],
    });
  },
}));

/// Models for an engine — used by the config form's model picker.
export function modelsFor(engine: "claude" | "codex") {
  return profileFor(engine).models;
}

export function teardownRoundtableListeners() {
  unlistenTurn?.();
  unlistenStatus?.();
  unlistenActivity?.();
  unlistenTurn = null;
  unlistenStatus = null;
  unlistenActivity = null;
  listenersBound = false;
}
