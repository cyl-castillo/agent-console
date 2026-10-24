import { Inbox as InboxIcon } from "lucide-react";
import { useSessionStore } from "../../stores/sessionStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useUIStore } from "../../stores/uiStore";
import { useWorkbenchStore } from "../../stores/workbenchStore";
import { useInbox } from "./useInbox";
import { ApprovalDecision } from "./ApprovalDecision";
import { BudgetDecision } from "./BudgetDecision";
import { DeclineDecision } from "./DeclineDecision";
import { ReviewDecision } from "./ReviewDecision";
import { VerificationDecision } from "./VerificationDecision";
import { TrustDecision } from "./TrustDecision";
import { PrDecision } from "./PrDecision";
import { tierRates, useDecisionStatsStore } from "../../stores/decisionStatsStore";
import { Tooltip } from "../../ui/Tooltip";
import { Button } from "../../ui/Button";
import { Chip } from "../../ui/Chip";
import { OnboardingHint } from "./OnboardingHint";
import { useOnboardingSteps } from "../useOnboardingSteps";
import { useOverlaysStore } from "../../stores/overlaysStore";
import { useSponsorStore } from "../../stores/sponsorStore";
import { EmptyState } from "../../ui/EmptyState";
import { Row } from "../../ui/Row";
import { SectionHeader } from "../../ui/SectionHeader";
import { StatusGlyph, type AgentState } from "../../ui/StatusGlyph";
import { newSession } from "../actions";

function relative(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  return h < 48 ? `${h}h` : `${Math.round(h / 24)}d`;
}

/// The decision queue (D4). F2 ships its skeleton on the data the app already
/// has: pending approvals, sessions grouped by attention state, jobs that
/// need a human. The per-decision card with tiers, previews and budgets
/// arrives in F3.
export function InboxView() {
  const {
    now,
    approvals: queue,
    jobsNeedingYou,
    pendingJobs,
    byState,
    sessions,
    activeId,
    breaches,
    declines,
    reviews,
    verifications,
    pr,
    trust,
    decisions,
  } = useInbox();
  const project = useSessionStore((s) => s.project);
  const records = useDecisionStatsStore((s) => s.records);
  const rates = tierRates(records, now).filter((r) => r.total > 0);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  const setCenterTab = useUIStore((s) => s.setTab);
  const visible = sessions;
  const onboarding = useOnboardingSteps();
  const sponsor = useSponsorStore((s) => s.sponsor);
  const showOverlay = useOverlaysStore((s) => s.show);

  const openSession = (id: string) => {
    const s = useTerminalsStore.getState().sessions.find((x) => x.id === id);
    if (s?.status === "stopped") useTerminalsStore.getState().resume(id);
    else setActive(id);
    goTo("sessions");
    setCenterTab("terminal");
  };
  const openJobs = () => {
    setWorkbenchTab("jobs");
  };

  const groups: { state: AgentState; title: string }[] = [
    { state: "needs-you", title: "Needs you" },
    { state: "working", title: "Working" },
    { state: "idle", title: "Idle" },
    { state: "stopped", title: "Stopped" },
  ];

  if (decisions === 0 && visible.length === 0) {
    return (
      <EmptyState
        className="h-full"
        icon={<InboxIcon />}
        title="Nothing to decide"
        description="Start a session and the agent's questions, approvals and finished work will land here."
        action={
          <div className="flex items-center gap-2">
            <Button variant="primary" onClick={newSession}>
              New session
            </Button>
            {onboarding.pending > 0 && !onboarding.dismissed && (
              <Button variant="ghost" onClick={() => showOverlay("gettingStarted")}>
                Open the guide
              </Button>
            )}
          </div>
        }
        checklist={onboarding.pending > 0 && !onboarding.dismissed ? onboarding.steps : undefined}
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto" data-testid="inbox">
      <OnboardingHint />
      <div className="flex items-center gap-2 border-b border-border px-4 py-2 font-sans">
        <h1 className="m-0 text-title font-semibold text-foreground">Inbox</h1>
        <Chip tone={decisions > 0 ? "warn" : "neutral"} mono>
          {decisions} to decide
        </Chip>
        {rates.map((r) => (
          <Tooltip
            key={r.tier}
            content={
              r.decayed
                ? `You approved ${Math.round((r.rate ?? 0) * 100)}% of ${r.total} ${r.tier}-tier decisions in 14 days — this gate has decayed into a reflex. Consider an Always rule, or look closer.`
                : `${r.total} ${r.tier}-tier decisions in 14 days, ${Math.round((r.rate ?? 0) * 100)}% approved`
            }
          >
            <span>
              <Chip tone={r.decayed ? "warn" : "neutral"} mono>
                {r.tier} {Math.round((r.rate ?? 0) * 100)}%
              </Chip>
            </span>
          </Tooltip>
        ))}
        {sponsor?.autoApprove && (
          <Tooltip content="The sponsor policy decides low-risk requests alone; you see what crosses the threshold. Work → Jobs to change it.">
            <span>
              <Chip tone="ok" mono>
                sponsor ≤ {sponsor.riskThreshold}
              </Chip>
            </span>
          </Tooltip>
        )}
        <span className="ml-auto" />
        <Button size="sm" onClick={newSession}>
          New session
        </Button>
      </div>

      {trust && (
        <section>
          <SectionHeader title="Trust" count={1} />
          <TrustDecision status={trust} projectName={project?.name ?? "this folder"} />
        </section>
      )}

      {pr && (
        <section>
          <SectionHeader title="Pull request" count={1} />
          <PrDecision ov={pr.overview} why={pr.why} />
        </section>
      )}

      {queue.length > 0 && (
        <section>
          <SectionHeader title="Approvals" count={queue.length} />
          {queue.map((r, i) => (
            <ApprovalDecision key={r.id} req={r} index={i} total={queue.length} />
          ))}
        </section>
      )}

      {reviews.length > 0 && (
        <section>
          <SectionHeader title="Reviews" count={reviews.length} />
          {reviews.map((r) => (
            <ReviewDecision key={r.id} r={r} />
          ))}
        </section>
      )}

      {verifications.length > 0 && (
        <section>
          <SectionHeader title="Verification" count={verifications.length} />
          {verifications.map((t) => (
            <VerificationDecision key={t.turnId ?? t.ts} turn={t} />
          ))}
        </section>
      )}

      {declines.length > 0 && (
        <section>
          <SectionHeader title="Declined / escalated" count={declines.length} />
          {declines.map((d) => (
            <DeclineDecision key={d.id} d={d} />
          ))}
        </section>
      )}

      {breaches.length > 0 && (
        <section>
          <SectionHeader title="Budget" count={breaches.length} />
          {breaches.map((b) => (
            <BudgetDecision key={b.session.id} session={b.session} spent={b.spent} cap={b.cap} />
          ))}
        </section>
      )}

      {(jobsNeedingYou.length > 0 || pendingJobs.length > 0) && (
        <section>
          <SectionHeader title="Jobs" count={jobsNeedingYou.length + pendingJobs.length} />
          {jobsNeedingYou.map((c) => (
            <Row
              key={c.id}
              leading={<StatusGlyph state="needs-you" />}
              title={c.problem}
              subtitle={
                c.reason ??
                (c.status === "awaiting_confirmation" ? "ready to land — confirm" : c.phase)
              }
              trailing={relative(c.updatedAtMs, now)}
              onClick={openJobs}
            />
          ))}
          {pendingJobs.map((p) => (
            <Row
              key={p.pending.id}
              leading={<StatusGlyph state="review" />}
              title={p.pending.instructions}
              subtitle={`proposed in "${p.sourceProblem}"`}
              trailing={relative(p.pending.createdMs, now)}
              onClick={openJobs}
            />
          ))}
        </section>
      )}

      {groups.map(({ state, title }) => {
        // A session with a pending approval is already on screen as a card.
        const carded = new Set(queue.map((r) => r.termId));
        const list = (byState.get(state) ?? []).filter(
          (s) => !(state === "needs-you" && carded.has(s.id)),
        );
        if (list.length === 0) return null;
        return (
          <section key={state}>
            <SectionHeader title={title} count={list.length} />
            {list.map((s) => (
              <Row
                key={s.id}
                leading={<StatusGlyph state={state} />}
                title={s.name}
                subtitle={
                  s.worktree
                    ? s.worktree.branch
                    : s.agent
                      ? `${s.agent}${s.model ? ` · ${s.model}` : ""}`
                      : undefined
                }
                trailing={relative(s.lastActiveMs ?? s.createdAtMs, now)}
                selected={s.id === activeId}
                onClick={() => openSession(s.id)}
                data-testid={`inbox-session-${s.id}`}
              />
            ))}
          </section>
        );
      })}
    </div>
  );
}
