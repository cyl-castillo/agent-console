import { useState } from "react";
import { ipc } from "../../ipc/tauri";
import { evidencePrompt } from "../../lib/verificationInbox";
import { typeIntoActiveSession } from "../../lib/termInput";
import type { TimelineTurn } from "../../stores/proofStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useToastStore } from "../../stores/toastStore";
import { useUIStore } from "../../stores/uiStore";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";
import { Textarea } from "../../ui/Field";
import type { DecisionTier } from "../../lib/decisions";
import { verificationGlyph, verificationLabel, verificationTone } from "./conversation/checkStatus";

const TIER: Record<string, { tier: DecisionTier; label: string; reason: string }> = {
  claim_contradicted: {
    tier: "medium",
    label: "Claim contradicted",
    reason:
      "the agent said green over a check that failed or ran nothing; accepting needs a reason",
  },
  stale_check: {
    tier: "low",
    label: "Stale check",
    reason: "files changed after the last passing check; the claim is not covered",
  },
  unverified_claim: {
    tier: "low",
    label: "Unverified claim",
    reason: "the agent said green and no passing check was witnessed",
  },
};

/// F1d: the console's verification verdict as a decision. Ask for evidence
/// types the project's gate into the agent (the next turn's verdict answers);
/// Request review hands the turn to the reviewer agent; Accept as is records
/// the human's word (with a reason when the evidence contradicted the claim).
export function VerificationDecision({ turn }: { turn: TimelineTurn }) {
  const v = turn.verification!;
  const project = useSessionStore((s) => s.project);
  const sessions = useTerminalsStore((s) => s.sessions);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setCenterTab = useUIStore((s) => s.setTab);
  const showToast = useToastStore((s) => s.show);
  const [accepting, setAccepting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  // F9: "que lo revise otro" — the human may name the reviewer's engine;
  // the backend refuses one that is the producer.
  const [reviewBy, setReviewBy] = useState("auto");
  const session = sessions.find((s) => s.id === turn.termId);
  const t = TIER[v.status] ?? TIER.unverified_claim;
  const needsReason = v.status === "claim_contradicted";
  const root = project?.root ?? "";
  const turnId = turn.turnId ?? "";

  const record = async (decision: "accept" | "evidence_requested", why?: string) => {
    await ipc.verificationDecide(
      root,
      turn.termId ?? "",
      turnId,
      turn.caseId ?? "",
      v.seq,
      decision,
      why,
    );
  };

  const askForEvidence = async () => {
    setBusy(true);
    try {
      if (session) {
        setActive(session.id);
        goTo("sessions");
        setCenterTab("terminal");
      }
      const sent = await typeIntoActiveSession(evidencePrompt(turn), { submit: true });
      await record("evidence_requested");
      showToast(
        sent
          ? "Asked the agent to run the gate — the next turn's verdict answers"
          : "Prompt copied: the session is not live, paste it when it is",
        sent ? "success" : "info",
      );
    } catch (e) {
      showToast(`Couldn't ask for evidence: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const requestReview = async () => {
    setBusy(true);
    try {
      await ipc.reviewTurn(
        root,
        turn.termId ?? "",
        turnId,
        reviewBy === "auto" ? undefined : { engine: reviewBy },
      );
      showToast(
        `Reviewer asked${reviewBy === "auto" ? "" : ` (${reviewBy})`} — its verdict lands in the Inbox`,
        "success",
      );
    } catch (e) {
      showToast(`Couldn't request a review: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const accept = async () => {
    setBusy(true);
    try {
      await record("accept", reason.trim() || undefined);
    } catch (e) {
      showToast(`Couldn't record the decision: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const actions = accepting
    ? [
        { label: "Back", variant: "ghost" as const, onClick: () => setAccepting(false) },
        {
          label: "Record acceptance",
          variant: "primary" as const,
          disabled: needsReason && reason.trim().length < 8,
          onClick: () => void accept(),
        },
      ]
    : [
        {
          label: "Accept as is",
          variant: (needsReason ? "danger" : "secondary") as "danger" | "secondary",
          onClick: () => (needsReason ? setAccepting(true) : void accept()),
        },
        {
          label: "Request review",
          variant: "secondary" as const,
          onClick: () => void requestReview(),
        },
        {
          label: "Ask for evidence",
          variant: "primary" as const,
          onClick: () => void askForEvidence(),
        },
      ];

  const notRun = Object.entries(v.gates).filter(([, s]) => s === "not_run");
  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`verification-${turnId}`}
        tier={t.tier}
        tierLabel={t.label}
        tierReason={t.reason}
        busy={busy}
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip tone={verificationTone(v.status)} mono>
              {verificationGlyph(v.status)} {verificationLabel(v.status)}
            </Chip>
            {v.claims.length > 0 ? `“${v.claims[0]}”` : "a green claim"}
          </span>
        }
        context={`${session?.name ?? turn.termId ?? "session"} · turn ${turnId.slice(0, 8)} · ${v.checks.passed} passed · ${v.checks.failed} failed · ${v.checks.empty} ran nothing${notRun.length ? ` · not run: ${notRun.map(([k]) => k).join(", ")}` : ""} · computed from witnessed lines`}
        preview={
          <div className="flex flex-col gap-1.5 px-2 py-1.5 font-sans text-small text-foreground">
            <p className="m-0">{v.reason}</p>
            {turn.summary && (
              <p className="m-0 text-muted-foreground">
                <span className="text-faint-foreground">Agent: </span>
                {turn.summary}
              </p>
            )}
            {!accepting && (
              <label className="flex items-center gap-2 text-caption text-muted-foreground">
                review by
                <select
                  aria-label="Review by"
                  data-testid="review-by"
                  className="rounded-1 border border-border bg-background px-1 text-caption text-foreground"
                  value={reviewBy}
                  onChange={(e) => setReviewBy(e.target.value)}
                >
                  <option value="auto">another engine (auto)</option>
                  <option value="claude">claude</option>
                  <option value="codex">codex</option>
                  <option value="opencode">opencode</option>
                </select>
              </label>
            )}
            {accepting && (
              <Textarea
                aria-label="Why you accept the claim without evidence"
                placeholder="Why the claim is acceptable here (goes into the ledger)"
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
