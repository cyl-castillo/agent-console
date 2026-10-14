import { useEffect, useState } from "react";
import { useAgentStatusStore } from "../../stores/agentStatusStore";
import { useApprovalStore } from "../../stores/approvalStore";
import { useJobsStore } from "../../stores/jobsStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { useSessionStore } from "../../stores/sessionStore";
import { capFor, useBudgetStore } from "../../stores/budgetStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { useProofStore } from "../../stores/proofStore";
import { useDeclinesStore, type Decline } from "../../stores/declinesStore";
import { sessionAttention } from "../../lib/attention";
import type { AgentState } from "../../ui/StatusGlyph";
import type { ApprovalRequest, JobCard, JobPendingCard } from "../../types/domain";

export interface InboxData {
  now: number;
  approvals: ApprovalRequest[];
  jobsNeedingYou: JobCard[];
  pendingJobs: JobPendingCard[];
  byState: Map<AgentState, TerminalSession[]>;
  sessions: TerminalSession[];
  activeId: string | null;
  /// Sessions whose cost crossed their cap (the turn was interrupted).
  breaches: { session: TerminalSession; spent: number; cap: number }[];
  /// Agents that declined or escalated, unacknowledged (D13).
  declines: Decline[];
  /// Everything that waits for a human decision — the number on the rail
  /// and in the Inbox header are this same count.
  decisions: number;
}

/// One derivation of "what needs a decision" for the rail badge, the status
/// bar and the Inbox itself.
export function useInbox(): InboxData {
  const queue = useApprovalStore((s) => s.queue);
  const waiting = useAgentStatusStore((s) => s.waiting);
  const workingUntil = useAgentStatusStore((s) => s.workingUntil);
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const board = useJobsStore((s) => s.board);
  const budgets = useBudgetStore();
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const events = useProofStore((s) => s.events);
  const project = useSessionStore((s) => s.project);
  const allDeclines = useDeclinesStore((s) => s.items);
  // A clock so relative ages move without a store write; the working window
  // needs it too.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 15_000);
    return () => window.clearInterval(id);
  }, []);

  const visible = sessions.filter((s) => !s.archived);
  // Sessions whose latest ledger turn is still open and recent.
  const OPEN_TURN_FRESH_MS = 15 * 60_000;
  const lastTurn = new Map<string, { ended: boolean; ts: number }>();
  for (const e of events) {
    if (!e.termId || !e.turnId) continue;
    if (e.kind === "prompt") lastTurn.set(e.termId, { ended: false, ts: e.ts });
    else {
      const t = lastTurn.get(e.termId);
      if (t) {
        t.ts = e.ts;
        if (e.kind === "turn_end") t.ended = true;
      }
    }
  }
  const openTurnIds = new Set(
    [...lastTurn.entries()]
      .filter(([, t]) => !t.ended && now - t.ts < OPEN_TURN_FRESH_MS)
      .map(([id]) => id),
  );
  const approvalTermIds = queue.map((r) => r.termId).filter((t): t is string => Boolean(t));
  const byState = new Map<AgentState, TerminalSession[]>();
  for (const s of visible) {
    const st = sessionAttention({
      session: s,
      approvalTermIds,
      waitingTermId: waiting?.termId ?? null,
      workingUntil,
      workingTermId: null,
      activeId,
      now,
      openTurn: openTurnIds.has(s.id),
    });
    byState.set(st, [...(byState.get(st) ?? []), s]);
  }
  const jobsNeedingYou = (board?.cards ?? []).filter(
    (c) => c.status === "needs_attention" || c.status === "awaiting_confirmation",
  );
  const pendingJobs = board?.pending ?? [];
  // Sessions already counted through an approval are not counted twice.
  const needsYouWithoutApproval = (byState.get("needs-you") ?? []).filter(
    (s) => !approvalTermIds.includes(s.id),
  ).length;
  const breaches = visible
    .filter((s) => budgets.breached[s.id] !== undefined)
    .map((session) => ({
      session,
      spent: byTerm[session.id]?.costUsd ?? budgets.breached[session.id],
      cap: capFor(budgets, session.id),
    }));
  const declines = allDeclines.filter(
    (d) => !d.resolved && (!project || d.project === project.root),
  );
  const decisions =
    queue.length +
    jobsNeedingYou.length +
    pendingJobs.length +
    needsYouWithoutApproval +
    breaches.length +
    declines.length;

  return {
    now,
    approvals: queue,
    jobsNeedingYou,
    pendingJobs,
    byState,
    sessions: visible,
    activeId,
    breaches,
    declines,
    decisions,
  };
}
