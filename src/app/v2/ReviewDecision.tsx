import { useState } from "react";
import { ipc } from "../../ipc/tauri";
import { useReviewsStore, type Review } from "../../stores/reviewsStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useToastStore } from "../../stores/toastStore";
import { useUIStore } from "../../stores/uiStore";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";
import type { DecisionTier } from "../../lib/decisions";
import { Textarea } from "../../ui/Field";

const TIER: Record<string, { tier: DecisionTier; label: string; reason: string }> = {
  block: {
    tier: "high",
    label: "Reviewer blocks",
    reason: "a block stands until you override it with a reason",
  },
  request_changes: {
    tier: "medium",
    label: "Reviewer requests changes",
    reason: "the reviewer found defects; the producer has not seen them",
  },
  approve: { tier: "low", label: "Reviewer approves", reason: "nothing to act on; acknowledge" },
};

/// D10: the reviewer agent's verdict as a decision. Accept keeps the verdict
/// as it stands (a block keeps blocking the PR check); Override needs a
/// reason and, for a block, the typed confirmation of a high-tier decision.
/// Both answers are witnessed as `review_decision`.
export function ReviewDecision({ r }: { r: Review }) {
  const v = r.verdict;
  const resolve = useReviewsStore((s) => s.resolve);
  const sessions = useTerminalsStore((s) => s.sessions);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setCenterTab = useUIStore((s) => s.setTab);
  const showToast = useToastStore((s) => s.show);
  const [overriding, setOverriding] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const session = sessions.find((s) => s.id === v.termId);
  const t = TIER[v.verdict] ?? TIER.request_changes;
  const top = v.findings[0];

  const decide = async (decision: "accept" | "override") => {
    setBusy(true);
    try {
      await ipc.reviewDecide(
        r.project,
        v.termId,
        v.turnId,
        v.caseId,
        v.seq ?? 0,
        decision,
        decision === "override" ? reason.trim() : undefined,
      );
      resolve(r.id);
    } catch (e) {
      showToast(`Couldn't record the decision: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const actions = overriding
    ? [
        { label: "Back", variant: "ghost" as const, onClick: () => setOverriding(false) },
        {
          label: "Record override",
          variant: "primary" as const,
          disabled: reason.trim().length < 8,
          onClick: () => void decide("override"),
        },
      ]
    : [
        {
          label: "Override",
          variant: "danger" as const,
          onClick: () => setOverriding(true),
        },
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
        {
          label: v.verdict === "approve" ? "Acknowledge" : "Accept verdict",
          variant: "primary" as const,
          onClick: () => void decide("accept"),
        },
      ];

  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`review-${r.id}`}
        tier={t.tier}
        tierLabel={t.label}
        tierReason={t.reason}
        typedConfirm={overriding && v.verdict === "block"}
        busy={busy}
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip tone="purple" mono>
              agent:{v.reviewer}
            </Chip>
            <Chip
              tone={v.verdict === "block" ? "danger" : v.verdict === "approve" ? "ok" : "warn"}
              mono
            >
              {v.verdict}
            </Chip>
            {top ? top.title : "No findings"}
          </span>
        }
        context={`${session?.name ?? v.termId} · turn ${v.turnId.slice(0, 8)} · ${v.files} files · reviewed by ${v.reviewerEngine}${v.reviewerModel ? ` (${v.reviewerModel})` : ""}${v.producerModel ? ` vs ${v.producerModel}` : ""} · recorded in the ledger`}
        preview={
          <div className="flex flex-col gap-1.5 px-2 py-1.5 font-sans text-small text-foreground">
            {v.findings.map((f, i) => (
              <div key={i} className="flex items-start gap-2">
                <Chip
                  tone={
                    f.severity === "block" ? "danger" : f.severity === "high" ? "warn" : "neutral"
                  }
                  mono
                >
                  {f.severity}
                </Chip>
                <span>
                  <span className="font-medium">{f.title}</span>
                  {f.deterministic && (
                    <span className="text-faint-foreground"> · rule {f.rule}</span>
                  )}
                  {f.file && <span className="font-code text-faint-foreground"> · {f.file}</span>}
                  <br />
                  <span className="text-muted-foreground">{f.detail}</span>
                </span>
              </div>
            ))}
            {v.rationale && (
              <p className="m-0 text-muted-foreground">
                <span className="text-faint-foreground">Reviewer: </span>
                {v.rationale}
              </p>
            )}
            {overriding && (
              <Textarea
                aria-label="Why you override the reviewer"
                placeholder="Why the verdict is wrong or acceptable here (goes into the ledger)"
                value={reason}
                rows={2}
                onChange={(e) => setReason(e.target.value)}
              />
            )}
          </div>
        }
        actions={actions}
      />
    </div>
  );
}
