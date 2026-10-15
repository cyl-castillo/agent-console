import { useBudgetStore } from "../../stores/budgetStore";
import { useSessionStore } from "../../stores/sessionStore";
import { recordBudget } from "../../lib/budgetGuard";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { useShellStore } from "../../stores/shellStore";
import { useUIStore } from "../../stores/uiStore";
import type { TermInputDetail } from "../../components/Terminal";
import { DecisionCard } from "../../ui/DecisionCard";
import { BudgetRing } from "../../ui/BudgetRing";

function sendToSession(sessionId: string, data: string) {
  const detail: TermInputDetail = { sessionId, data };
  window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
}

/// `budget_reached` as a decision: the turn was interrupted when the cap was
/// crossed; nothing resumes until you raise the cap, stop the session or ask
/// for a summary — and the answer is witnessed as `budget_decision` next to
/// the pause. The three multipliers behind a surprise are named.
export function BudgetDecision({
  session,
  spent,
  cap,
}: {
  session: TerminalSession;
  spent: number;
  cap: number;
}) {
  const setCap = useBudgetStore((s) => s.setCap);
  const root = useSessionStore((s) => s.project?.root);
  const clearBreach = useBudgetStore((s) => s.clearBreach);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setCenterTab = useUIStore((s) => s.setTab);
  const raise = Math.max(5, Math.ceil(cap * 0.5));
  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`budget-${session.id}`}
        tier="medium"
        tierLabel="Budget reached"
        tierReason="the turn was interrupted; the agent is waiting"
        intent={
          <span className="flex items-center gap-2">
            {session.name}
            <BudgetRing spent={spent} cap={cap} />
          </span>
        }
        context="Cost grows with extended thinking, the newer tokenizer (~30 % more tokens) and parallel subagents — a cap is the only hard stop."
        actions={[
          {
            label: "Stop session",
            variant: "danger",
            onClick: () => {
              void recordBudget(root, session, spent, cap, "stop");
              sendToSession(session.id, "\x03");
              clearBreach(session.id);
            },
          },
          {
            label: "Ask for a summary",
            variant: "secondary",
            onClick: () => {
              void recordBudget(root, session, spent, cap, "summary", cap + 1);
              setCap(session.id, cap + 1);
              sendToSession(session.id, "Summarize what you did and what is left, then stop.\r");
              setActive(session.id);
              goTo("sessions");
              setCenterTab("terminal");
            },
          },
          {
            label: `Raise to $${cap + raise}`,
            variant: "primary",
            onClick: () => {
              void recordBudget(root, session, spent, cap, "raise", cap + raise);
              setCap(session.id, cap + raise);
            },
          },
        ]}
      />
    </div>
  );
}
