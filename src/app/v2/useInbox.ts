import { useEffect, useState } from "react";
import { useAgentStatusStore } from "../../stores/agentStatusStore";
import { useApprovalStore } from "../../stores/approvalStore";
import { useJobsStore } from "../../stores/jobsStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { useSessionStore } from "../../stores/sessionStore";
import { capFor, useBudgetStore } from "../../stores/budgetStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { buildTimeline, useProofStore, type TimelineTurn } from "../../stores/proofStore";
import { pendingVerifications } from "../../lib/verificationInbox";
import { trustNeedsDecision } from "../../lib/trust";
import { useTrustStore } from "../../stores/trustStore";
import type { PrOverview, TrustStatus } from "../../types/domain";
import { prNeedsYou } from "../../lib/prInbox";
import { usePrStore } from "../../stores/prStore";
import { useDeclinesStore, type Decline } from "../../stores/declinesStore";
import { useReviewsStore, type Review } from "../../stores/reviewsStore";
import { sessionAttention } from "../../lib/attention";
import type { AgentState } from "../../ui/StatusGlyph";
import type { ApprovalRequest, JobCard, JobPendingCard } from "../../types/domain";

export interface InboxData {
  now: number;
  approvals: ApprovalRequest[];
  jobsNeedingYou: JobCard[];
  pendingJobs: JobPendingCard[];
  /// Each group newest-activity first.
  byState: Map<AgentState, TerminalSession[]>;
  /// Last activity per session: the later of the store's stamp and the
  /// session's last ledger line — the age in the list and its order.
  lastActive: Map<string, number>;
  sessions: TerminalSession[];
  activeId: string | null;
  /// Sessions whose cost crossed their cap (the turn was interrupted).
  breaches: { session: TerminalSession; spent: number; cap: number }[];
  /// Agents that declined or escalated, unacknowledged (D13).
  declines: Decline[];
  reviews: Review[];
  /// F1d: closed turns whose verification verdict waits for a human word.
  verifications: TimelineTurn[];
  /// F4: the open folder waits for the human's word before agents launch.
  trust: TrustStatus | null;
  /// F7: the branch's PR when it is a decision (CI red, fresh review
  /// comments, or green and waiting for the merge word).
  pr: { overview: PrOverview; why: "ci_red" | "comments" | "ready" } | null;
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
  const workingTermId = useAgentStatusStore((s) => s.workingTermId);
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const board = useJobsStore((s) => s.board);
  const budgets = useBudgetStore();
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const events = useProofStore((s) => s.events);
  const project = useSessionStore((s) => s.project);
  const allDeclines = useDeclinesStore((s) => s.items);
  const allReviews = useReviewsStore((s) => s.items);
  const trustStatus = useTrustStore((s) => s.status);
  const prOverview = usePrStore((s) => s.overview);
  const prDismissed = usePrStore((s) => s.dismissed);
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
  const lastEvent = new Map<string, number>();
  for (const e of events) {
    if (e.termId) lastEvent.set(e.termId, Math.max(lastEvent.get(e.termId) ?? 0, e.ts));
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
  const lastActive = new Map<string, number>();
  for (const s of sessions) {
    lastActive.set(s.id, Math.max(s.lastActiveMs ?? s.createdAtMs, lastEvent.get(s.id) ?? 0));
  }
  const byState = new Map<AgentState, TerminalSession[]>();
  for (const s of visible) {
    const st = sessionAttention({
      session: s,
      approvalTermIds,
      waitingTermId: waiting?.termId ?? null,
      workingUntil,
      workingTermId,
      activeId,
      now,
      openTurn: openTurnIds.has(s.id),
    });
    byState.set(st, [...(byState.get(st) ?? []), s]);
  }
  for (const list of byState.values()) {
    list.sort((a, b) => (lastActive.get(b.id) ?? 0) - (lastActive.get(a.id) ?? 0));
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
  const reviews = allReviews.filter((r) => !r.resolved && (!project || r.project === project.root));
  const verifications = pendingVerifications(buildTimeline(events));
  const trust = trustNeedsDecision(trustStatus) ? trustStatus : null;
  const prWhy = prNeedsYou(prOverview, events, prDismissed);
  const pr = prWhy && prOverview ? { overview: prOverview, why: prWhy } : null;
  const decisions =
    queue.length +
    (pr ? 1 : 0) +
    reviews.length +
    verifications.length +
    (trust ? 1 : 0) +
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
    lastActive,
    sessions: visible,
    activeId,
    breaches,
    declines,
    reviews,
    verifications,
    trust,
    pr,
    decisions,
  };
}
