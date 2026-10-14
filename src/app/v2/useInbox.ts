import { useEffect, useState } from "react";
import { useAgentStatusStore } from "../../stores/agentStatusStore";
import { useApprovalStore } from "../../stores/approvalStore";
import { useJobsStore } from "../../stores/jobsStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { capFor, useBudgetStore } from "../../stores/budgetStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
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
  // A clock so relative ages move without a store write; the working window
  // needs it too.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 15_000);
    return () => window.clearInterval(id);
  }, []);

  const visible = sessions.filter((s) => !s.archived);
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
  const decisions =
    queue.length +
    jobsNeedingYou.length +
    pendingJobs.length +
    needsYouWithoutApproval +
    breaches.length;

  return {
    now,
    approvals: queue,
    jobsNeedingYou,
    pendingJobs,
    byState,
    sessions: visible,
    activeId,
    breaches,
    decisions,
  };
}
