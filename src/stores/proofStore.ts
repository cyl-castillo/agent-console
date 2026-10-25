import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { profileFor } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import { sessionLedgerCost } from "../lib/usageStats";
import { useChangesStore } from "./changesStore";
import { useLiveStatusStore } from "./liveStatusStore";
import { useTerminalsStore } from "./terminalsStore";
import { useToastStore } from "./toastStore";
import type {
  ProofEvent,
  TestigoVerifyReport,
  TestigoExportSummary,
  AatExportSummary,
  AatVerifyReport,
  ComplianceSummary,
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
  /// True when the turn closed through Codex's Interrupt — the user cut it
  /// short. Not a failure (nothing refused it) and not a finish either; the
  /// diff still shows what it had changed by then.
  interrupted: boolean;
  /// True when the turn never got its own close: the next prompt on the same
  /// terminal ended it (Esc in Claude has no hook; a killed session fires
  /// nothing). Always `interrupted` too.
  superseded: boolean;
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
  /// D5: the turn was interrupted at its budget cap (`budget_reached`), and
  /// what the human answered (`budget_decision`), when any.
  budget?: { spentUsd: number; capUsd: number; decision?: string; newCapUsd?: number };
  /// D10: the reviewer agent's verdict on this turn (`review_verdict`) and
  /// the human's answer (`review_decision`), when any.
  review?: { verdict: string; findings: number; reviewer: string; decision?: string };
  /// F1c: the console's verdict on the turn's own claims (`verification`),
  /// computed from witnessed lines the moment the turn closed.
  verification?: TimelineVerification;
  /// F2: what the turn cost, from the engine's own numbers (turn_end.usage).
  usage?: TimelineUsage;
}

export interface TimelineUsage {
  /// claude · codex · opencode
  engine: string;
  /// statusline · rollout · plugin
  source: string;
  /// Session cost so far (Claude's status line) — cumulative.
  costUsd?: number;
  /// This turn's cost: the delta (Claude) or the turn's own sum (OpenCode).
  costTurnUsd?: number;
  tokensIn?: number;
  tokensOut?: number;
  cacheRead?: number;
  model?: string;
  quota?: { fiveHourPct?: number; sevenDayPct?: number; spendPct?: number };
}

export interface TimelineVerification {
  /// Ledger seq of the verification line (a decision references it).
  seq: number;
  /// verified · unverified_claim · claim_contradicted · stale_check ·
  /// checked · no_claim · unwitnessed
  status: string;
  reason: string;
  /// per_tool · turn_boundary
  coverage: string;
  checks: { passed: number; failed: number; empty: number; unknown: number; interrupted: number };
  /// Green claims found in the closing words (negated ones excluded).
  claims: string[];
  /// kind → status (passed · failed · empty · unknown · interrupted · not_run).
  gates: Record<string, string>;
  /// kind → the command the project's manifest names for it, when known.
  gateCommands: Record<string, string>;
  /// F1d: the human's answer (`verification_decision`), when any.
  decision?: { decision: string; reason?: string };
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
  /** passed · failed · interrupted · empty (exit 0, ran nothing) · unknown (no exit code seen). */
  status: "passed" | "failed" | "interrupted" | "empty" | "unknown" | string;
  /** test · lint · typecheck · build · format (what the runner checks). */
  kind?: string;
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
  /// D3: last IETF audit-trail export plus the console's own re-verification
  /// of every chain it wrote (a third party runs the same check offline).
  lastAat: { summary: AatExportSummary; reports: [string, AatVerifyReport][] } | null;
  /// F11: the last compliance pack written.
  lastCompliance: ComplianceSummary | null;
  error: string | null;
  /// Case opened in the timeline view; null = case list.
  selectedCase: string | null;
  /// Pre-sign review in progress (null = none). `undefined` caseId inside it
  /// means "full ledger".
  review: { caseId?: string; preview: TestigoExportPreview; redactSeqs: number[] } | null;
  /// Per-project policy (witness local / repo marks opt-in).
  settings: TestigoSettings | null;

  load: (projectRoot: string) => Promise<void>;
  /// Re-read the events only (no chain verification, no settings): what the
  /// live listener calls on every appended line. Cheap enough to run often.
  refresh: (projectRoot: string) => Promise<void>;
  /// Show the ledger the session running in `cwd` writes to (its worktree or
  /// linked folder may file evidence apart from the project root). Same rule
  /// as the hook events, resolved by the backend.
  followCheckout: (cwd: string) => Promise<void>;
  clear: () => void;
  /// Step 1: open the pre-sign review for a case (or the full ledger).
  startExport: (caseId?: string) => Promise<void>;
  /// Export a case (or the ledger) as IETF Agent Audit Trail JSONL and
  /// re-verify what was written. Nothing is signed; no review step needed —
  /// the records carry the same payloads the ledger already holds locally.
  exportAat: (caseId?: string) => Promise<void>;
  compliancePack: (caseId?: string) => Promise<void>;
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
  // The turn still open on each terminal: a terminal runs one turn at a
  // time, so a new prompt there ends whatever was open (older ledgers hold
  // such turns without a close; newer ones close them on the Rust side).
  const openByTerm = new Map<string, TimelineTurn>();
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
        interrupted: false,
        superseded: false,
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
      if (e.termId) {
        const open = openByTerm.get(e.termId);
        if (open && open !== t && open.endTs === null) {
          open.endTs = e.ts;
          open.interrupted = true;
          open.superseded = true;
        }
        openByTerm.set(e.termId, t);
      }
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
        status: typeof p.status === "string" ? p.status : "unknown",
        kind: typeof p.kind === "string" ? p.kind : undefined,
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
    } else if (e.kind === "review_verdict") {
      t.review = {
        verdict: typeof p.verdict === "string" ? p.verdict : "approve",
        findings: Array.isArray(p.findings) ? p.findings.length : 0,
        reviewer: e.actor,
      };
    } else if (e.kind === "review_decision") {
      if (t.review) t.review.decision = typeof p.decision === "string" ? p.decision : undefined;
    } else if (e.kind === "budget_reached") {
      t.budget = {
        spentUsd: typeof p.spentUsd === "number" ? p.spentUsd : 0,
        capUsd: typeof p.capUsd === "number" ? p.capUsd : 0,
      };
    } else if (e.kind === "budget_decision") {
      t.budget = {
        spentUsd: typeof p.spentUsd === "number" ? p.spentUsd : (t.budget?.spentUsd ?? 0),
        capUsd: typeof p.capUsd === "number" ? p.capUsd : (t.budget?.capUsd ?? 0),
        decision: typeof p.decision === "string" ? p.decision : undefined,
        newCapUsd: typeof p.newCapUsd === "number" ? p.newCapUsd : undefined,
      };
    } else if (e.kind === "verification") {
      const checks = (p.checks ?? {}) as Record<string, unknown>;
      const num = (k: string) => (typeof checks[k] === "number" ? (checks[k] as number) : 0);
      const gates: Record<string, string> = {};
      const gateCommands: Record<string, string> = {};
      if (p.gates && typeof p.gates === "object") {
        for (const [k, v] of Object.entries(
          p.gates as Record<string, { status?: unknown; command?: unknown }>,
        )) {
          if (typeof v?.status === "string") gates[k] = v.status;
          if (typeof v?.command === "string") gateCommands[k] = v.command;
        }
      }
      t.verification = {
        seq: e.seq,
        status: typeof p.status === "string" ? p.status : "no_claim",
        reason: typeof p.reason === "string" ? p.reason : "",
        coverage: typeof p.coverage === "string" ? p.coverage : "per_tool",
        checks: {
          passed: num("passed"),
          failed: num("failed"),
          empty: num("empty"),
          unknown: num("unknown"),
          interrupted: num("interrupted"),
        },
        claims: Array.isArray(p.claims)
          ? (p.claims as { phrase?: unknown; negated?: unknown }[])
              .filter((c) => c?.negated !== true && typeof c?.phrase === "string")
              .map((c) => c.phrase as string)
          : [],
        gates,
        gateCommands,
      };
    } else if (e.kind === "verification_decision") {
      if (t.verification) {
        t.verification.decision = {
          decision: typeof p.decision === "string" ? p.decision : "accept",
          reason: typeof p.reason === "string" ? p.reason : undefined,
        };
      }
    } else if (e.kind === "turn_end") {
      if (e.termId && openByTerm.get(e.termId) === t) openByTerm.delete(e.termId);
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
      t.interrupted = p.interrupted === true;
      t.superseded = p.superseded === true;
      if (typeof p.error === "string") t.error = p.error;
      if (typeof p.errorDetails === "string") t.errorDetails = p.errorDetails;
      if (typeof p.postSha === "string" && p.postSha) t.postSha = p.postSha;
      if (p.usage && typeof p.usage === "object") {
        const u = p.usage as Record<string, unknown>;
        const tk = (u.tokens ?? {}) as Record<string, unknown>;
        const num = (v: unknown) => (typeof v === "number" ? v : undefined);
        const q = (u.quota ?? {}) as Record<string, unknown>;
        const quota = {
          fiveHourPct: num(q.fiveHourPct),
          sevenDayPct: num(q.sevenDayPct),
          spendPct: num(q.spendPct),
        };
        t.usage = {
          engine: typeof u.engine === "string" ? u.engine : "unknown",
          source: typeof u.source === "string" ? u.source : "unknown",
          costUsd: num(u.costUsd),
          costTurnUsd: u.engine === "claude" ? num(u.costDeltaUsd) : num(u.costUsd),
          tokensIn: num(tk.input),
          tokensOut: num(tk.output),
          cacheRead: num(tk.cacheRead),
          model: typeof u.model === "string" ? u.model : undefined,
          quota:
            quota.fiveHourPct !== undefined ||
            quota.sevenDayPct !== undefined ||
            quota.spendPct !== undefined
              ? quota
              : undefined,
        };
      }
      // The close carries the binding too — keeps the turn actionable even
      // when the prompt event predates a hook that sent no ids.
      if (!t.termId && e.termId) t.termId = e.termId;
      if (!t.sessionId && e.sessionId) t.sessionId = e.sessionId;
    }
  }
  return turns;
}

/// Latest followCheckout call; an older resolution arriving late is dropped.
let followSeq = 0;

export const useProofStore = create<ProofState>((set, get) => ({
  projectRoot: null,
  events: [],
  report: null,
  exporting: null,
  lastExport: null,
  lastAat: null,
  lastCompliance: null,
  error: null,
  selectedCase: null,
  review: null,
  settings: null,

  load: async (projectRoot) => {
    // Another ledger: drop what belongs to the previous one instead of showing
    // it under the new name while the read is in flight.
    const switched = projectRoot !== get().projectRoot;
    set({
      projectRoot,
      error: null,
      ...(switched
        ? { events: [], report: null, settings: null, selectedCase: null, review: null }
        : {}),
    });
    try {
      const [events, report, settings] = await Promise.all([
        ipc.testigoList(projectRoot),
        ipc.testigoVerify(projectRoot),
        ipc.testigoGetSettings(projectRoot),
      ]);
      // A session switch while this was in flight wins.
      if (get().projectRoot !== projectRoot) return;
      set({ events, report, settings });
      feedLedgerCosts(events);
    } catch (e) {
      if (get().projectRoot !== projectRoot) return;
      set({ error: String(e) });
    }
  },

  followCheckout: async (cwd) => {
    const seq = ++followSeq;
    const root = await ipc.ledgerRootFor(cwd).catch(() => cwd);
    if (seq !== followSeq) return;
    await get().load(root);
  },

  clear: () =>
    set({
      projectRoot: null,
      events: [],
      report: null,
      lastExport: null,
      lastAat: null,
      lastCompliance: null,
      error: null,
      selectedCase: null,
      review: null,
    }),

  refresh: async (projectRoot) => {
    if (projectRoot !== get().projectRoot) return get().load(projectRoot);
    try {
      const events = await ipc.testigoList(projectRoot);
      if (get().projectRoot !== projectRoot) return;
      set({ events });
      feedLedgerCosts(events);
    } catch (e) {
      if (get().projectRoot !== projectRoot) return;
      set({ error: String(e) });
    }
  },

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

  exportAat: async (caseId) => {
    const root = get().projectRoot;
    if (!root) return;
    set({ exporting: caseId ?? "__ledger__", error: null });
    try {
      const summary = await ipc.testigoExportAat(root, caseId);
      const reports = await ipc.testigoVerifyAat(summary.dir);
      set({ lastAat: { summary, reports }, exporting: null });
    } catch (e) {
      set({ exporting: null, error: String(e) });
    }
  },
  compliancePack: async (caseId) => {
    const root = get().projectRoot;
    if (!root) return;
    set({ exporting: caseId ?? "__ledger__", error: null });
    try {
      const summary = await ipc.testigoCompliancePack(root, caseId);
      set({ lastCompliance: summary, exporting: null });
    } catch (e) {
      set({ exporting: null, error: String(e) });
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

/// F2: engines without a status line (OpenCode through its plugin) report
/// cost per turn in the ledger; their sessions' running total feeds the same
/// store the budget watch reads, so a cap pauses them too.
function feedLedgerCosts(events: ProofEvent[]) {
  const turns = buildTimeline(events);
  const live = useLiveStatusStore.getState();
  const seen = new Set<string>();
  for (const t of turns) {
    if (!t.termId || seen.has(t.termId) || t.usage?.engine === "claude") continue;
    seen.add(t.termId);
    const cost = sessionLedgerCost(turns, t.termId);
    if (cost !== undefined) live.noteLedgerCost(t.termId, cost, t.endTs ?? t.ts);
  }
}

/// Keep the ledger view live. The source of truth is `ledger://appended`,
/// which the Rust side emits AFTER each line is on disk — the hook events
/// (`hook://turn_end` and friends) fire before the slow parts (snapshot,
/// diff) and used to trigger a re-read of a ledger that did not yet hold the
/// line, so the conversation lagged until the next prompt. Those events stay
/// as a fallback; every trigger is debounced, and re-reads the events only
/// (the chain is verified on the full `load`, not on every line).
export const PROOF_RELOAD_DEBOUNCE_MS = 150;

export async function attachProofListeners(): Promise<UnlistenFn> {
  let timer: number | null = null;
  const schedule = () => {
    if (timer !== null) return;
    timer = window.setTimeout(() => {
      timer = null;
      const root = useProofStore.getState().projectRoot;
      if (root) void useProofStore.getState().refresh(root);
    }, PROOF_RELOAD_DEBOUNCE_MS);
  };
  const kinds = [
    "ledger://appended",
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
