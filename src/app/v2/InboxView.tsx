import { Inbox as InboxIcon } from "lucide-react";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useUIStore } from "../../stores/uiStore";
import { useWorkbenchStore } from "../../stores/workbenchStore";
import { useInbox } from "./useInbox";
import { Button } from "../../ui/Button";
import { Chip } from "../../ui/Chip";
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
    decisions,
  } = useInbox();
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  const setCenterTab = useUIStore((s) => s.setTab);
  const visible = sessions;

  const openSession = (id: string) => {
    setActive(id);
    goTo("sessions");
    setCenterTab("terminal");
  };
  const openJobs = () => {
    setWorkbenchTab("jobs");
  };

  const groups: { state: AgentState; title: string }[] = [
    { state: "needs-you", title: "Needs you" },
    { state: "working", title: "Working" },
    { state: "done", title: "Idle" },
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
          <Button variant="primary" onClick={newSession}>
            New session
          </Button>
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto" data-testid="inbox">
      <div className="flex items-center gap-2 border-b border-border px-4 py-2 font-sans">
        <h1 className="m-0 text-title font-semibold text-foreground">Inbox</h1>
        <Chip tone={decisions > 0 ? "warn" : "neutral"} mono>
          {decisions} to decide
        </Chip>
        <span className="ml-auto" />
        <Button size="sm" onClick={newSession}>
          New session
        </Button>
      </div>

      {queue.length > 0 && (
        <section>
          <SectionHeader title="Approvals" count={queue.length} />
          {queue.map((r) => {
            const sess = sessions.find((s) => s.id === r.termId);
            const cmd = typeof r.input?.command === "string" ? r.input.command : "";
            return (
              <Row
                key={r.id}
                leading={<StatusGlyph state="needs-you" />}
                title={`${r.tool}${cmd ? ` · ${cmd}` : ""}`}
                subtitle={sess ? sess.name : r.cwd}
                trailing={relative(r.ts, now)}
                onClick={() => r.termId && openSession(r.termId)}
                data-testid={`inbox-approval-${r.id}`}
              />
            );
          })}
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
        const list = byState.get(state) ?? [];
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
