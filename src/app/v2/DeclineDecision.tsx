import { useDeclinesStore, type Decline } from "../../stores/declinesStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useUIStore } from "../../stores/uiStore";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";

/// An agent declined or escalated (D13). The reason is the intent, the
/// suggestion the preview; the human acknowledges or opens the session. There
/// is deliberately no "retry" here.
export function DeclineDecision({ d }: { d: Decline }) {
  const resolve = useDeclinesStore((s) => s.resolve);
  const sessions = useTerminalsStore((s) => s.sessions);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setCenterTab = useUIStore((s) => s.setTab);
  const session = d.session ? sessions.find((s) => s.id === d.session) : undefined;
  const escalated = d.kind === "escalate";
  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`decline-${d.id}`}
        tier="medium"
        tierLabel={escalated ? "Escalated to you" : "Declined"}
        tierReason={
          escalated ? "the agent cannot finish without a human" : "the agent will not do this"
        }
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip tone="purple" mono>
              agent:{d.caller}
            </Chip>
            {d.reason}
          </span>
        }
        context={session ? `${session.name} · recorded in the ledger` : "recorded in the ledger"}
        preview={
          d.suggestion ? (
            <div className="px-2 py-1.5 font-sans text-small text-foreground">
              <span className="text-muted-foreground">Suggests: </span>
              {d.suggestion}
            </div>
          ) : undefined
        }
        actions={[
          ...(session
            ? [
                {
                  label: "Open session",
                  variant: "secondary" as const,
                  onClick: () => {
                    setActive(session.id);
                    goTo("sessions");
                    setCenterTab("terminal");
                  },
                },
              ]
            : []),
          { label: "Acknowledge", variant: "primary" as const, onClick: () => resolve(d.id) },
        ]}
      />
    </div>
  );
}
