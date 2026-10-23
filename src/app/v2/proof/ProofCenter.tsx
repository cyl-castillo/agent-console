import { useMemo } from "react";
import { fmtUsd, usageWeek } from "../../../lib/usageStats";
import { verificationStats } from "../../../lib/verificationInbox";
import { buildTimeline, useProofStore } from "../../../stores/proofStore";
import { useWorkbenchStore } from "../../../stores/workbenchStore";
import { Chip } from "../../../ui/Chip";
import { Tooltip } from "../../../ui/Tooltip";
import { WorkbenchPanel } from "../../panels";
import { TAB_LABELS } from "../destinations";
import { LedgerMinimap } from "./LedgerMinimap";

/// F2: what the agents cost this week, from the ledger's turn closes — by
/// engine on hover; Codex turns are counted but unpriced (tokens only).
function UsageWeekChip() {
  const events = useProofStore((s) => s.events);
  const week = useMemo(() => usageWeek(buildTimeline(events)), [events]);
  if (week.turns === 0) return null;
  const parts = Object.entries(week.byEngine).map(
    ([e, v]) =>
      `${e}: ${v.turns} turn${v.turns === 1 ? "" : "s"}, ${v.costUsd > 0 ? fmtUsd(v.costUsd) : `${v.tokensOut.toLocaleString()} output tokens, unpriced`}`,
  );
  return (
    <Tooltip
      content={`Last 7 days, from the engines' own numbers — ${parts.join(" · ")}${week.unpricedTurns ? ` · ${week.unpricedTurns} turn${week.unpricedTurns === 1 ? "" : "s"} without a dollar figure` : ""}`}
    >
      <span className="ml-2 inline-flex items-center" data-testid="usage-week">
        <Chip mono>
          {fmtUsd(week.costUsd)} · 7d{week.unpricedTurns ? " +tokens" : ""}
        </Chip>
      </span>
    </Tooltip>
  );
}

/// F1d habit metric: of the week's green claims, how many a passing check
/// after the last edit actually backed. The number to make go up.
function VerificationHabit() {
  const events = useProofStore((s) => s.events);
  const stats = useMemo(() => verificationStats(buildTimeline(events)), [events]);
  if (stats.claims === 0) return null;
  const pct = Math.round((stats.verified / stats.claims) * 100);
  return (
    <Tooltip
      content={`Last 7 days: ${stats.claims} green claim${stats.claims === 1 ? "" : "s"} — ${stats.verified} verified by a passing check after the last edit, ${stats.unverified} unverified or stale, ${stats.contradicted} contradicted by the evidence`}
    >
      <span className="ml-auto inline-flex items-center gap-1" data-testid="verification-habit">
        <Chip tone={pct >= 80 ? "ok" : pct >= 50 ? "warn" : "danger"} mono>
          verified {stats.verified}/{stats.claims}
        </Chip>
        {stats.contradicted > 0 && (
          <Chip tone="danger" mono>
            ✕ {stats.contradicted}
          </Chip>
        )}
      </span>
    </Tooltip>
  );
}

/// Proof destination: the map on top, the Turns or Ledger view below.
export function ProofCenter() {
  const tab = useWorkbenchStore((s) => s.tab);
  return (
    <section
      className="flex h-full min-h-0 flex-col"
      aria-label={TAB_LABELS[tab]}
      data-testid="center-panel"
    >
      <div className="flex h-8 shrink-0 items-center border-b border-border px-3 font-sans text-small font-semibold text-foreground">
        {TAB_LABELS[tab]}
        <UsageWeekChip />
        <VerificationHabit />
      </div>
      <LedgerMinimap />
      <div className="workbench-layout min-h-0 flex-1">
        <div className="workbench-content">
          <WorkbenchPanel tab={tab} />
        </div>
      </div>
    </section>
  );
}
