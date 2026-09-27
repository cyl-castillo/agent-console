import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { profileFor } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import { useChangesStore } from "./changesStore";
import { useTerminalsStore } from "./terminalsStore";
import { useToastStore } from "./toastStore";
import type {
  ProofEvent,
  TestigoVerifyReport,
  TestigoExportSummary,
  TestigoExportPreview,
  TestigoSettings,
} from "../types/domain";

/// One row of the case list: an intent thread with its activity envelope.
export interface CaseSummary {
  caseId: string;
  events: number;
  turns: number;
  approvals: number;
  lastTs: number;
}

/// One turn of a case timeline: intent → approvals → results, assembled from
/// the flat event stream.
export interface TimelineTurn {
  turnId: string | null;
  ts: number;
  prompt: string;
  skill?: string;
  approvals: { tool?: string; decision?: string; reason?: string }[];
  toolResults: number;
  files: { status: string; path: string }[];
  filesTruncated: boolean;
  endTs: number | null;
  /// What the agent said when it closed the turn, when the CLI reports it
  /// (Claude's `last_assistant_message`). Empty when it didn't.
  summary: string;
  summaryTruncated: boolean;
  /// True when the turn closed through StopFailure — the API refused it. The
  /// diff still shows what the turn had changed by then; this marks that it
  /// never finished on its own terms. `error` is Claude's reason enum.
  failed: boolean;
  error?: string;
  errorDetails?: string;
  /// Where the turn ran and what it left behind — everything "Rewind to this
  /// turn" needs: the terminal binding, the engine session to fork, the
  /// checkout (worktree sessions differ from the project root) and the
  /// post-turn snapshot to restore files to.
  termId?: string;
  sessionId?: string;
  cwd?: string;
  postSha?: string;
  /// True when a later rewind event points at this turn — the timeline shows
  /// where history was rewound to.
  rewound: boolean;
  /// Test/check runs the agent made inside the turn (`check_run` events,
  /// T4b): the "tests ran, and this is what they said" evidence a reviewer
  /// reads before any prompt.
  checks: TimelineCheck[];
  /// Commits the human made from the turn's diff (`commit` events, T4b).
  commits: TimelineCommit[];
  /// Every tool call recorded in the turn, in order (capped; `toolResults`
  /// keeps the true count). What the Turns view lists under a prompt.
  tools: TimelineTool[];
  /// Corpus doc ids the console injected into this prompt (`context_injected`).
  injected: string[];
  /// Model the session reported during the turn (session_start /
  /// model_switch), when any.
  model?: string;
  /// Case the turn belongs to (from its prompt event) — the Ledger view's key.
  caseId?: string;
}

export interface TimelineTool {
  tool: string;
  /// Bash: the command line. Other tools: absent.
  command?: string;
  /// Head of the tool's output (the ledger keeps ≤1 KB; the view shows less).
  excerpt?: string;
  failed: boolean;
  exitCode?: number;
  /// Ran inside a subagent (Task), not the session's main thread.
  agentId?: string;
}

/// Tool calls kept per turn in the timeline (the count stays exact).
export const TIMELINE_TOOLS_CAP = 60;

export interface TimelineCheck {
  command: string;
  status: "passed" | "failed" | "interrupted" | string;
  exitCode?: number;
  durationMs?: number;
}

export interface TimelineCommit {
  sha: string;
  subject: string;
  files: number;
  amend: boolean;
}

interface ProofState {
  projectRoot: string | null;
  events: ProofEvent[];
  report: TestigoVerifyReport | null;
  exporting: string | null;
  lastExport: TestigoExportSummary | null;
  error: string | null;
  /// Case opened in the timeline view; null = case list.
  selectedCase: string | null;
  /// Pre-sign review in progress (null = none). `undefined` caseId inside it
  /// means "full ledger".
  review: { caseId?: string; preview: TestigoExportPreview; redactSeqs: number[] } | null;
  /// Per-project policy (witness local / repo marks opt-in).
  settings: TestigoSettings | null;

  load: (projectRoot: string) => Promise<void>;
  clear: () => void;
  /// Step 1: open the pre-sign review for a case (or the full ledger).
  startExport: (caseId?: string) => Promise<void>;
  /// Toggle manual redaction of one event in the open review.
  toggleRedact: (seq: number) => void;
  /// Step 2: sign and write the packet with the chosen redactions.
  confirmExport: () => Promise<void>;
  cancelExport: () => void;
  selectCase: (caseId: string | null) => void;
  setSettings: (patch: Partial<TestigoSettings>) => Promise<void>;
  /// "Rewind to this turn": restore the turn's checkout to its post-turn
  /// snapshot, fork the agent's transcript truncated after the turn, and open
  /// a NEW session resuming the fork — the original session and transcript
  /// stay untouched as history. When the fork fails the files are restored
  /// anyway and the toast SAYS the memory was not rewound.
  rewindToTurn: (t: TimelineTurn) => Promise<void>;
}

export function summarizeCases(events: ProofEvent[]): CaseSummary[] {
  const by = new Map<string, CaseSummary>();
  for (const e of events) {
    const c = by.get(e.caseId) ?? {
      caseId: e.caseId,
      events: 0,
      turns: 0,
      approvals: 0,
      lastTs: 0,
    };
    c.events += 1;
    if (e.kind === "prompt") c.turns += 1;
    if (e.kind === "approval_decision") c.approvals += 1;
    if (e.ts > c.lastTs) c.lastTs = e.ts;
    by.set(e.caseId, c);
  }
  // Most recently active first.
  return [...by.values()].sort((a, b) => b.lastTs - a.lastTs);
}

/// Fold a case's events into turns (chronological). Events keyed by turnId;
/// a prompt opens the turn, turn_end closes it with the diff payload. Events
/// with no turn (case_link, job_run, post-restart strays) are skipped — the
/// case header already shows totals.
export function buildTimeline(events: ProofEvent[]): TimelineTurn[] {
  const turns: TimelineTurn[] = [];
  const byId = new Map<string, TimelineTurn>();
  const turnFor = (e: ProofEvent): TimelineTurn | null => {
    if (!e.turnId) return null;
    let t = byId.get(e.turnId);
    if (!t) {
      t = {
        turnId: e.turnId,
        ts: e.ts,
        prompt: "",
        approvals: [],
        toolResults: 0,
        files: [],
        filesTruncated: false,
        endTs: null,
        summary: "",
        summaryTruncated: false,
        failed: false,
        rewound: false,
        checks: [],
        commits: [],
        tools: [],
        injected: [],
      };
      byId.set(e.turnId, t);
      turns.push(t);
    }
    return t;
  };
  for (const e of events) {
    // A rewind event's turnId POINTS AT the turn it restored — it must mark
    // that turn, never open a phantom one (e.g. after the pointed-at turn was
    // exported away or the ledger only holds the tail).
    if (e.kind === "rewind") {
      const target = e.turnId ? byId.get(e.turnId) : undefined;
      if (target) target.rewound = true;
      continue;
    }
    const t = turnFor(e);
    if (!t) continue;
    const p = e.payload as Record<string, unknown>;
    if (e.kind === "prompt") {
      t.ts = e.ts;
      t.caseId = e.caseId;
      t.prompt = typeof p.prompt === "string" ? p.prompt : "";
      if (typeof p.skill === "string") t.skill = p.skill;
      if (e.termId) t.termId = e.termId;
      if (e.sessionId) t.sessionId = e.sessionId;
      if (typeof p.cwd === "string" && p.cwd) t.cwd = p.cwd;
    } else if (e.kind === "approval_decision") {
      t.approvals.push({
        tool: typeof p.tool === "string" ? p.tool : undefined,
        decision: typeof p.decision === "string" ? p.decision : undefined,
        reason: typeof p.reason === "string" ? p.reason : undefined,
      });
    } else if (e.kind === "tool_result") {
      t.toolResults += 1;
      if (t.tools.length < TIMELINE_TOOLS_CAP) {
        t.tools.push({
          tool: typeof p.tool === "string" ? p.tool : "tool",
          command: typeof p.command === "string" ? p.command : undefined,
          excerpt: typeof p.excerpt === "string" ? p.excerpt : undefined,
          failed: p.failed === true,
          exitCode: typeof p.exitCode === "number" ? p.exitCode : undefined,
          agentId: typeof p.agentId === "string" ? p.agentId : undefined,
        });
      }
    } else if (e.kind === "context_injected") {
      if (Array.isArray(p.docs)) {
        for (const d of p.docs) if (typeof d === "string") t.injected.push(d);
      }
    } else if (e.kind === "session_start") {
      if (typeof p.model === "string" && p.model) t.model = p.model;
    } else if (e.kind === "model_switch") {
      if (typeof p.to === "string" && p.to) t.model = p.to;
    } else if (e.kind === "check_run") {
      t.checks.push({
        command: typeof p.command === "string" ? p.command : "",
        status: typeof p.status === "string" ? p.status : "passed",
        exitCode: typeof p.exitCode === "number" ? p.exitCode : undefined,
        durationMs: typeof p.durationMs === "number" ? p.durationMs : undefined,
      });
    } else if (e.kind === "commit") {
      t.commits.push({
        sha: typeof p.sha === "string" ? p.sha : "",
        subject: typeof p.subject === "string" ? p.subject : "",
        files: Array.isArray(p.files) ? p.files.length : 0,
        amend: p.amend === true,
      });
    } else if (e.kind === "turn_end") {
      t.endTs = e.ts;
      if (Array.isArray(p.filesChanged)) {
        t.files = (p.filesChanged as { status: string; path: string }[]).filter(
          (f) => typeof f?.path === "string",
        );
      }
      t.filesTruncated = p.filesTruncated === true;
      if (typeof p.summary === "string") t.summary = p.summary;
      t.summaryTruncated = p.summaryTruncated === true;
      t.failed = p.failed === true;
      if (typeof p.error === "string") t.error = p.error;
      if (typeof p.errorDetails === "string") t.errorDetails = p.errorDetails;
      if (typeof p.postSha === "string" && p.postSha) t.postSha = p.postSha;
      // The close carries the binding too — keeps the turn actionable even
      // when the prompt event predates a hook that sent no ids.
      if (!t.termId && e.termId) t.termId = e.termId;
      if (!t.sessionId && e.sessionId) t.sessionId = e.sessionId;
    }
  }
  return turns;
}

export const useProofStore = create<ProofState>((set, get) => ({
  projectRoot: null,
  events: [],
  report: null,
  exporting: null,
  lastExport: null,
  error: null,
  selectedCase: null,
  review: null,
  settings: null,

  load: async (projectRoot) => {
    set({ projectRoot, error: null });
    try {
      const [events, report, settings] = await Promise.all([
        ipc.testigoList(projectRoot),
        ipc.testigoVerify(projectRoot),
        ipc.testigoGetSettings(projectRoot),
      ]);
      set({ events, report, settings });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  clear: () =>
    set({
      projectRoot: null,
      events: [],
      report: null,
      lastExport: null,
      error: null,
      selectedCase: null,
      review: null,
    }),

  selectCase: (caseId) => set({ selectedCase: caseId }),

  startExport: async (caseId) => {
    const root = get().projectRoot;
    if (!root) return;
    set({ error: null });
    try {
      const preview = await ipc.testigoExportPreview(root, caseId);
      set({ review: { caseId, preview, redactSeqs: [] } });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  toggleRedact: (seq) =>
    set((s) => {
      if (!s.review) return s;
      const has = s.review.redactSeqs.includes(seq);
      return {
        review: {
          ...s.review,
          redactSeqs: has
            ? s.review.redactSeqs.filter((x) => x !== seq)
            : [...s.review.redactSeqs, seq],
        },
      };
    }),

  confirmExport: async () => {
    const { projectRoot: root, review } = get();
    if (!root || !review) return;
    set({ exporting: review.caseId ?? "__ledger__", error: null });
    try {
      const lastExport = await ipc.testigoExport(root, review.caseId, undefined, review.redactSeqs);
      set({ lastExport, exporting: null, review: null });
    } catch (e) {
      set({ exporting: null, error: String(e) });
    }
  },

  cancelExport: () => set({ review: null }),

  rewindToTurn: async (t) => {
    // Re-checked even though the button already gates: this action rewrites a
    // working tree, and the ledger can lag the session list.
    if (!t.termId || !t.sessionId || !t.postSha || t.endTs == null) return;
    const terminals = useTerminalsStore.getState();
    const src = terminals.sessions.find((s) => s.id === t.termId);
    // The engine gate lives on the profile (Claude-only today): a turn whose
    // session is gone can't name its engine, so it isn't rewindable either.
    if (!src || !profileFor(src.agent).supportsTranscriptFork) return;
    // A live agent would keep writing over the restored tree from its
    // un-rewound conversation — the UI disables the button, this is the belt.
    if (src.status === "live") return;
    const cwd = t.cwd ?? src.cwd;
    try {
      const res = await ipc.turnRewind({
        repo: cwd,
        commitSha: t.postSha,
        sessionId: t.sessionId,
        cutoffMs: t.endTs,
        termId: t.termId,
        turnId: t.turnId ?? undefined,
      });
      await useChangesStore.getState().refresh();
      if (res.forkSessionId) {
        // New session in the turn's checkout, resuming the forked (rewound)
        // conversation. Bind the fork id in the same tick as add(): the
        // terminal's spawn effect reads it when building `--resume`.
        const newId = terminals.add(
          cwd,
          `${src.name} ↶`,
          src.model,
          src.agent,
          src.worktree && src.worktree.path === cwd ? src.worktree : undefined,
        );
        terminals.setAgentSessionId(newId, res.forkSessionId);
        useToastStore
          .getState()
          .show(
            "Rewound: files restored, new session resumes the conversation as of that turn",
            "success",
          );
      } else {
        // Honest degradation, loud on purpose (error tone persists): files
        // moved but the agent still remembers the turns that produced them —
        // exactly the desync the user asked to undo.
        useToastStore
          .getState()
          .show(
            `Files restored, but the agent's memory was NOT rewound — the conversation still remembers later turns. ${res.forkError ?? ""}`,
            "error",
          );
      }
      // Reload so the timeline shows the rewind event on the turn.
      const root = get().projectRoot;
      if (root) void get().load(root);
    } catch (e) {
      useToastStore.getState().show(`Rewind failed: ${e}`, "error");
    }
  },

  setSettings: async (patch) => {
    const { projectRoot: root, settings } = get();
    if (!root || !settings) return;
    const next = { ...settings, ...patch };
    // Optimistic; revert on failure.
    set({ settings: next });
    try {
      const saved = await ipc.testigoSetSettings(root, next);
      set({ settings: saved });
    } catch (e) {
      set({ settings, error: String(e) });
    }
  },
}));

/// Keep the ledger view live: every hook-borne event that lands in the
/// ledger re-reads it (debounced — a turn can emit a dozen tool results in
/// a second). Until now Proof loaded on project open and on a manual ↻; the
/// Turns view is watched while the agent works, so it has to move by itself.
export const PROOF_RELOAD_DEBOUNCE_MS = 400;

export async function attachProofListeners(): Promise<UnlistenFn> {
  let timer: number | null = null;
  const schedule = () => {
    if (timer !== null) return;
    timer = window.setTimeout(() => {
      timer = null;
      const root = useProofStore.getState().projectRoot;
      if (root) void useProofStore.getState().load(root);
    }, PROOF_RELOAD_DEBOUNCE_MS);
  };
  const kinds = [
    "hook://user_prompt",
    "hook://tool_result",
    "hook://tool_failed",
    "hook://turn_end",
    "hook://turn_failed",
    "hook://model_switch",
    "hook://approval_deferred",
    "approval://request",
    "snapshot://created",
  ];
  const offs: UnlistenFn[] = [];
  for (const k of kinds) offs.push(await listen(k, schedule));
  return () => {
    if (timer !== null) window.clearTimeout(timer);
    for (const off of offs) off();
  };
}
