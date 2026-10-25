import { useState } from "react";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";
import type { TimelineTurn } from "../../stores/proofStore";
import type { TerminalSession } from "../../stores/terminalsStore";
import { HandoffPicker } from "./HandoffAction";
import { useHandoff } from "./useHandoff";

/// F10: the session's last turn died on the plan's quota (StopFailure
/// `rate_limit`). The decision: wait for the window to reset, or continue
/// the work on an open-weight model through OpenCode — the hand-over is
/// witnessed as `model_switch {reason: quota}` and the card leaves.
export function QuotaDecision({ session, turn }: { session: TerminalSession; turn: TimelineTurn }) {
  const handoff = useHandoff(session, turn, "quota");
  const [dismissed, setDismissed] = useState(false);
  if (dismissed) return null;
  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`quota-${session.id}`}
        tier="medium"
        tierLabel="Quota exhausted"
        tierReason="the plan's usage window is used up; the last turn was refused"
        busy={handoff.busy}
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip tone="warn" mono>
              rate_limit
            </Chip>
            {session.name}
          </span>
        }
        context={`${session.agent ?? "claude"}${session.model ? ` (${session.model})` : ""} · turn ${turn.turnId?.slice(0, 8) ?? "?"} · ${turn.files.length} file${turn.files.length === 1 ? "" : "s"} touched`}
        preview={
          <div className="flex flex-col gap-1.5 px-2 py-1.5 font-sans text-small text-foreground">
            {turn.summary && <p className="m-0 text-muted-foreground">{turn.summary}</p>}
            <HandoffPicker handoff={handoff} testid={`quota-model-${session.id}`} />
          </div>
        }
        actions={[
          {
            label: "Wait for the window",
            variant: "ghost",
            onClick: () => setDismissed(true),
          },
          {
            label: "Continue in OpenCode",
            variant: "primary",
            disabled: !handoff.chosen,
            onClick: () => void handoff.run(),
          },
        ]}
      />
    </div>
  );
}
