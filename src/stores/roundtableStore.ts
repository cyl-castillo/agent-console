import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { useChangesStore } from "./changesStore";
import { profileFor } from "../agents/profiles";
import type {
  ConnectorView,
  RoomSummary,
  RoundtableActivity,
  RoundtableJobEvent,
  RoundtableConfig,
  RoundtableParticipant,
  RoundtableStatus,
  RoundtableTurn,
} from "../types/domain";

export type RtPhase = "config" | "running" | "paused" | "awaiting" | "done" | "stopped" | "error";

/// A roster entry edited in the config form before launch.
export interface RtParticipantDraft {
  id: string;
  name: string;
  engine: "claude" | "codex";
  model: string;
  role: string;
  /// Connector roles (see CONNECTOR_ROLES). Empty = plain assistant.
  roles: string[];
}

/// The connector roles a participant can hold. `assistant` is the implicit
/// default and is never listed.
export const CONNECTOR_ROLES = [
  "organizer",
  "implementer",
  "reviewer",
  "planner",
  "consultant",
] as const;

/// Draft of the room setup, edited in the config form before launch.
export interface RtDraft {
  problem: string;
  participants: RtParticipantDraft[];
  maxTurns: number;
  tokenBudget: number;
  /// Working room: let agents edit the code in an isolated worktree.
  allowEdits: boolean;
  /// Job mode: the organizer drives the work through the connector.
  jobMode: boolean;
  /// Job mode: a reviewer must approve before the job closes.
  reviewRequired: boolean;
  /// Job mode: "changes" verdicts absorbed before the job stops for you.
  maxCorrections: number;
  /// Working-room jobs: wait for your confirmation before landing (default).
  confirmLanding: boolean;
}

const DEFAULT_DRAFT: RtDraft = {
  problem: "",
  participants: [
    { id: "p1", name: "Opus", engine: "claude", model: "opus", role: "", roles: ["organizer"] },
    { id: "p2", name: "Codex", engine: "codex", model: "medium", role: "", roles: ["implementer"] },
  ],
  maxTurns: 6,
  // A soft checkpoint, not a wall — when reached the room pauses and you can
  // continue (the backend grants another window). Opus rooms burn through tokens
  // fast, so start generous; cache reads are excluded from the count.
  tokenBudget: 1_000_000,
  allowEdits: false,
  jobMode: false,
  reviewRequired: false,
  maxCorrections: 2,
  confirmLanding: true,
};

interface RoundtableState {
  runId: string | null;
  phase: RtPhase;
  /// True when viewing a saved room re-hydrated from disk: the feed is frozen,
  /// no controls act on a live run, and reset/close never discards from disk.
  readOnly: boolean;
  /// The problem the displayed room is working on (live run or saved view).
  /// Kept separate from `draft.problem` so opening a saved room never clobbers
  /// an in-progress config draft.
  problem: string;
  /// Saved rooms for the open project, newest first — the sidebar list.
  rooms: RoomSummary[];
  turn: number;
  /// The current turn target (grows with each Continue) — the "/N" in the meta.
  targetTurns: number;
  totalTokens: number;
  approxCostUsd: number;
  message: string | null;
  /// The shared conversation, in order (agent turns + human messages).
  turns: RoundtableTurn[];
  /// Live activity lines (reasoning, tool calls, streamed text) keyed by
  /// authorId+turn.
  activities: RoundtableActivity[];

  injectDraft: string;
  /// True when the displayed room is a working room (agents edit a room/<id>
  /// branch) — drives the cowork bar. Tracked separately from `draft.allowEdits`
  /// (the config form) so it's correct for a reopened room too: set from the
  /// launched config on start and from the persisted flag on open.
  workingRoom: boolean;
  /// Cowork (share/sync with human colleagues over the git remote). `busy` marks
  /// an in-flight share/sync so the buttons disable; `result` holds the last
  /// outcome to show inline (PR link on share, conflicts on sync).
  coworkBusy: "share" | "sync" | null;
  coworkResult:
    | { kind: "share"; message: string; prUrl: string | null }
    | { kind: "sync"; message: string; conflicts: string[] }
    | null;
  /// Wall-clock (ms) the current live AI turn began. Drives the per-turn clock.
  liveStartedAt: number | null;
  /// Wall-clock (ms) of the most recent activity event — the staleness signal.
  lastActivityAt: number | null;
  /// What the agents did through the connector (delegations, questions,
  /// reviews) for the displayed room. Refreshed on every turn and every MCP call.
  connector: ConnectorView | null;
  /// Free-text draft for the pending question's answer.
  answerDraft: string;
  /// True while an answer is being submitted.
  answering: boolean;
  /// True when the displayed room runs in job mode (live config or saved flag).
  jobMode: boolean;
  /// Latest job phase reported by the backend (`roundtable://job`); null until
  /// the first event, then the panel prefers it over its derived guess.
  jobPhase: RoundtableJobEvent | null;
  /// Follow-up job: the room this one was approved from.
  originRoomId: string | null;

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
  continueRoom: () => Promise<void>;
  /// Push this working room's branch to the remote and hand back an MR/PR link.
  share: () => Promise<void>;
  /// Pull a colleague's commits from the remote room branch into the worktree.
  sync: () => Promise<void>;
  /// Dismiss the inline cowork result banner.
  clearCowork: () => void;
  stop: () => Promise<void>;
  reset: () => Promise<void>;
  loadRooms: () => Promise<void>;
  openRoom: (id: string) => Promise<void>;
  deleteSavedRoom: (id: string) => Promise<void>;
  resumeRoom: () => Promise<void>;
  loadConnector: () => Promise<void>;
  setAnswerDraft: (v: string) => void;
  /// Answer the pending question: by option id, or with the free-text draft.
  answerQuestion: (questionId: string, choiceId: string | null) => Promise<void>;
  /// Approve or discard a task an agent left waiting (`create_task`).
  resolvePending: (pendingId: string, approve: boolean) => Promise<void>;
  /// Show a job from the board LIVE: load its saved transcript and keep
  /// listening to its events (unlike openRoom, which freezes read-only).
  attachRoom: (id: string, status: string) => Promise<void>;
}

// Promise-singleton so concurrent initListeners() calls (panel mount + start)
// share one bind, and teardown can await an in-flight bind before unlistening.
let bindPromise: Promise<void> | null = null;
let unlistenTurn: UnlistenFn | null = null;
let unlistenStatus: UnlistenFn | null = null;
let unlistenActivity: UnlistenFn | null = null;
let unlistenConnector: UnlistenFn | null = null;
let unlistenJob: UnlistenFn | null = null;
let pidCounter = 2;

export const useRoundtableStore = create<RoundtableState>((set, get) => ({
  runId: null,
  phase: "config",
  readOnly: false,
  problem: "",
  rooms: [],
  turn: 0,
  targetTurns: 0,
  totalTokens: 0,
  approxCostUsd: 0,
  message: null,
  turns: [],
  activities: [],

  injectDraft: "",
  workingRoom: false,
  coworkBusy: null,
  coworkResult: null,
  liveStartedAt: null,
  lastActivityAt: null,
  connector: null,
  answerDraft: "",
  answering: false,
  jobMode: false,
  jobPhase: null,
  originRoomId: null,

  draft: {
    ...DEFAULT_DRAFT,
    participants: DEFAULT_DRAFT.participants.map((p) => ({ ...p, roles: [...p.roles] })),
  },
  roster: [],

  initListeners: () => {
    bindPromise ??= bindListeners(set, get);
    return bindPromise;
  },

  setDraft: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),

  addParticipant: () =>
    set((s) => {
      pidCounter += 1;
      const id = `p${pidCounter}`;
      const next: RtParticipantDraft = {
        id,
        name: `Agent ${s.draft.participants.length + 1}`,
        engine: "claude",
        model: "sonnet",
        role: "",
        roles: [],
      };
      return { draft: { ...s.draft, participants: [...s.draft.participants, next] } };
    }),

  removeParticipant: (id) =>
    set((s) => {
      // A room needs at least two voices.
      if (s.draft.participants.length <= 2) return s;
      return {
        draft: { ...s.draft, participants: s.draft.participants.filter((p) => p.id !== id) },
      };
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
      roles: p.roles,
    }));
    // Honest working-room flag: editing needs a git repo to branch + review
    // against. Match the config toggle's own `!noRepo` guard (RoundtablePanel)
    // and the backend's `allow_edits = branch.is_some()`, so we never light up
    // the CoworkBar (Share) on a room that silently degraded to read-only.
    const allowEdits = d.allowEdits && useChangesStore.getState().status?.isRepo !== false;
    const config: RoundtableConfig = {
      problem: d.problem.trim(),
      participants,
      maxTurns: Math.max(1, Math.min(60, d.maxTurns)),
      tokenBudget: Math.max(0, d.tokenBudget),
      allowEdits,
      jobMode: d.jobMode,
      reviewRequired: d.jobMode && d.reviewRequired,
      maxCorrections: Math.max(0, Math.min(10, d.maxCorrections)),
      closure: d.confirmLanding ? "confirm" : "auto",
    };
    set({
      turns: [],
      activities: [],
      message: null,
      readOnly: false,
      workingRoom: allowEdits,
      problem: config.problem,
      totalTokens: 0,
      approxCostUsd: 0,
      turn: 0,
      targetTurns: config.maxTurns,
      roster: participants,
      phase: "running",
      liveStartedAt: Date.now(),
      lastActivityAt: null,
      connector: null,
      answerDraft: "",
      jobMode: d.jobMode,
      jobPhase: null,
      originRoomId: null,
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

  /// Run another round (one turn per participant), continuing the same
  /// conversation. Used from the "awaiting" state to keep it going.
  continueRoom: async () => {
    const id = get().runId;
    if (!id) return;
    const extra = Math.max(1, get().roster.length);
    set((s) => ({
      phase: "running",
      targetTurns: s.targetTurns + extra,
      liveStartedAt: Date.now(),
      lastActivityAt: null,
    }));
    try {
      await ipc.roundtableContinue(id, extra);
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  /// Share the room with colleagues: push its branch to the remote (transcript
  /// committed alongside) and surface the MR/PR link inline.
  share: async () => {
    const id = get().runId;
    if (!id || get().coworkBusy) return;
    set({ coworkBusy: "share", coworkResult: null });
    try {
      const r = await ipc.roundtableShare(id);
      set({ coworkResult: { kind: "share", message: r.message, prUrl: r.prUrl } });
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    } finally {
      set({ coworkBusy: null });
    }
  },

  /// Bring a colleague's commits into the live worktree so the next turn builds
  /// on top. Surfaces any conflicts inline (the backend aborts cleanly on them).
  sync: async () => {
    const id = get().runId;
    if (!id || get().coworkBusy) return;
    set({ coworkBusy: "sync", coworkResult: null });
    try {
      const r = await ipc.roundtableSync(id);
      set({ coworkResult: { kind: "sync", message: r.message, conflicts: r.conflicts } });
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    } finally {
      set({ coworkBusy: null });
    }
  },

  clearCowork: () => set({ coworkResult: null }),

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
    const { runId, readOnly, jobMode } = get();
    // A saved room being viewed lives only on disk — closing it must NOT discard
    // it. Only a live conversation run's in-memory record is dropped via the
    // backend. A JOB keeps running in the background: the panel merely detaches
    // (it stays reachable from the Jobs board; Close there ends it). Discarding
    // here is what closed a running job the moment "+ New job" was pressed.
    if (runId && !readOnly && !jobMode) {
      try {
        await ipc.roundtableDiscard(runId);
      } catch {
        /* best effort */
      }
    }
    set({
      runId: null,
      phase: "config",
      readOnly: false,
      problem: "",
      turn: 0,
      targetTurns: 0,
      totalTokens: 0,
      approxCostUsd: 0,
      message: null,
      turns: [],
      activities: [],
      injectDraft: "",
      workingRoom: false,
      coworkBusy: null,
      coworkResult: null,
      liveStartedAt: null,
      lastActivityAt: null,
      roster: [],
      connector: null,
      answerDraft: "",
      answering: false,
      jobMode: false,
      jobPhase: null,
      originRoomId: null,
    });
  },

  /// Refresh the saved-room list for the open project. Best-effort: with no
  /// project open (or a read error) the list is simply empty.
  loadRooms: async () => {
    try {
      set({ rooms: await ipc.roundtableListRooms() });
    } catch {
      set({ rooms: [] });
    }
  },

  /// Open a saved room read-only: fetch its full transcript and re-hydrate the
  /// feed frozen. Never resumes the engines — the conversation is just replayed.
  openRoom: async (id) => {
    try {
      const room = await ipc.roundtableGetRoom(id);
      if (!room) {
        await get().loadRooms();
        return;
      }
      const turns: RoundtableTurn[] = room.transcript.map((m, i) => ({
        id: `${room.id}-${i}`,
        authorId: m.authorId,
        authorName: m.authorName,
        engine: m.engine,
        model: m.model,
        text: m.text,
        turn: m.turn,
        isHuman: m.authorId === "human",
        totalTokens: 0,
        costUsd: 0,
        kind: m.kind ?? "",
      }));
      const lastTurn = room.transcript.reduce((mx, m) => Math.max(mx, m.turn), 0);
      set({
        runId: room.id,
        readOnly: true,
        workingRoom: room.allowEdits,
        phase: "done",
        problem: room.problem,
        roster: room.participants,
        turns,
        activities: [],
        turn: lastTurn,
        targetTurns: lastTurn,
        totalTokens: room.totalTokens,
        approxCostUsd: 0,
        message: null,
        injectDraft: "",
        liveStartedAt: null,
        lastActivityAt: null,
        connector: null,
        answerDraft: "",
        jobMode: !!room.job,
        jobPhase: room.job?.done
          ? {
              id: room.id,
              phase: "completed",
              corrections: 0,
              maxCorrections: room.job.maxCorrections,
            }
          : null,
        originRoomId: room.originRoomId ?? null,
      });
      void get().loadConnector();
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },

  /// Delete a saved room from disk. If it's the one currently being viewed, drop
  /// back to the config form.
  deleteSavedRoom: async (id) => {
    try {
      await ipc.roundtableDeleteRoom(id);
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
    if (get().runId === id && get().readOnly) await get().reset();
    await get().loadRooms();
  },

  /// Continue a saved room (Fase B): rebuild it as a live run on the backend and
  /// leave the panel in the "awaiting" state — the human adds a message and/or
  /// hits Continue to run more turns. The conversation, roster and token total
  /// already loaded by openRoom are kept; only the read-only lock is lifted.
  resumeRoom: async () => {
    const id = get().runId;
    if (!id || !get().readOnly) return;
    try {
      await get().initListeners();
      await ipc.roundtableResumeRoom(id);
      set({
        readOnly: false,
        phase: "awaiting",
        message:
          "Resumed from disk. Each agent's prior session may have expired — if so it starts fresh on its next turn.",
        liveStartedAt: null,
        lastActivityAt: null,
      });
    } catch (err) {
      set({ readOnly: true, message: err instanceof Error ? err.message : String(err) });
    }
  },

  /// Refresh the connector view for the displayed room. Best-effort: a read
  /// error leaves the last known view in place.
  loadConnector: async () => {
    const id = get().runId;
    if (!id) return;
    try {
      const view = await ipc.roundtableConnectorState(id);
      if (get().runId === id) set({ connector: view });
    } catch {
      /* keep the last view */
    }
  },

  setAnswerDraft: (v) => set({ answerDraft: v }),

  attachRoom: async (id, status) => {
    await get().openRoom(id);
    if (get().runId !== id) return;
    await get().initListeners();
    const phase: RtPhase =
      status === "running"
        ? "running"
        : status === "paused"
          ? "paused"
          : status === "completed"
            ? "done"
            : status === "closed"
              ? "stopped"
              : "awaiting";
    set({
      readOnly: false,
      phase,
      liveStartedAt: phase === "running" ? Date.now() : null,
      lastActivityAt: null,
    });
  },

  answerQuestion: async (questionId, choiceId) => {
    const id = get().runId;
    if (!id || get().answering) return;
    const body = choiceId ? "" : get().answerDraft.trim();
    if (!choiceId && !body) return;
    set({ answering: true });
    try {
      await ipc.roundtableAnswerQuestion(id, questionId, body, choiceId);
      set({ answerDraft: "", phase: "running", liveStartedAt: Date.now(), lastActivityAt: null });
      await get().loadConnector();
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    } finally {
      set({ answering: false });
    }
  },

  /// Approve starts the follow-up as its own job room with the same team and
  /// switches the panel to it live (the source keeps running or stays saved);
  /// discard just drops the proposal.
  resolvePending: async (pendingId, approve) => {
    const id = get().runId;
    if (!id) return;
    try {
      const resolved = await ipc.roundtableResolvePending(id, pendingId, approve);
      if (approve && resolved.approvedJobId) {
        await get().initListeners();
        const s = get();
        set({
          runId: resolved.approvedJobId,
          readOnly: false,
          phase: "running",
          problem: resolved.instructions,
          roster: s.roster,
          turns: [],
          activities: [],
          message: `Follow-up job started from this room's proposal.`,
          injectDraft: "",
          totalTokens: 0,
          approxCostUsd: 0,
          turn: 0,
          targetTurns: s.targetTurns || s.draft.maxTurns,
          liveStartedAt: Date.now(),
          lastActivityAt: null,
          connector: null,
          answerDraft: "",
          jobMode: true,
          jobPhase: null,
          originRoomId: id,
        });
        void get().loadRooms();
      }
      await get().loadConnector();
    } catch (err) {
      set({ message: err instanceof Error ? err.message : String(err) });
    }
  },
}));

/// Models for an engine — used by the config form's model picker.
export function modelsFor(engine: "claude" | "codex") {
  return profileFor(engine).models;
}

async function bindListeners(
  set: (
    partial: Partial<RoundtableState> | ((s: RoundtableState) => Partial<RoundtableState>),
  ) => void,
  get: () => RoundtableState,
): Promise<void> {
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
    // A finished turn may have settled a delegation or asked a question.
    void get().loadConnector();
  });
  // Every MCP call an agent makes mid-turn (delegate_task, ask_user, …) lands
  // here, so the delegations list grows while the turn is still running.
  unlistenConnector = await listen<{ job: string }>("connector://call", (e) => {
    if (e.payload.job !== get().runId) return;
    void get().loadConnector();
  });
  unlistenJob = await listen<RoundtableJobEvent>("roundtable://job", (e) => {
    if (e.payload.id !== get().runId) return;
    set({ jobPhase: e.payload });
  });
  unlistenStatus = await listen<RoundtableStatus>("roundtable://status", (e) => {
    const st = e.payload;
    if (st.id !== get().runId) return;
    const known: RtPhase[] = ["running", "paused", "awaiting", "done", "stopped", "error"];
    const phase = known.includes(st.status as RtPhase) ? (st.status as RtPhase) : get().phase;
    set({
      phase,
      turn: st.turn || get().turn,
      totalTokens: st.totalTokens || get().totalTokens,
      message: st.message ?? get().message,
    });
    if (phase === "awaiting") void get().loadConnector();
  });
}

export async function teardownRoundtableListeners() {
  // Wait out an in-flight bind so we never unlisten before listen resolves.
  const inFlight = bindPromise;
  bindPromise = null;
  if (inFlight) await inFlight.catch(() => {});
  unlistenTurn?.();
  unlistenStatus?.();
  unlistenActivity?.();
  unlistenConnector?.();
  unlistenJob?.();
  unlistenTurn = null;
  unlistenStatus = null;
  unlistenActivity = null;
  unlistenConnector = null;
  unlistenJob = null;
}
