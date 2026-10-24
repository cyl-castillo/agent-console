import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ipc } from "../../ipc/tauri";
import {
  autofixPrompt,
  ciLabel,
  ciTone,
  evidenceLabel,
  failedChecks,
  mergeGate,
  type PrCi,
} from "../../lib/prInbox";
import { typeIntoActiveSession } from "../../lib/termInput";
import { prKey, usePrStore } from "../../stores/prStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useToastStore } from "../../stores/toastStore";
import { useUIStore } from "../../stores/uiStore";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";
import { Textarea } from "../../ui/Field";
import type { DecisionTier } from "../../lib/decisions";
import type { PrOverview } from "../../types/domain";

/// F7: the pull request as a decision. Auto-fix types the review comments
/// into the session that produced the branch (the human sends; no bot of
/// ours answers the PR). Merge when green needs no word when every turn
/// behind the branch is verified; otherwise it needs yours, with a reason
/// — and the ledger carries which.
export function PrDecision({ ov, why }: { ov: PrOverview; why: "ci_red" | "comments" | "ready" }) {
  const pr = ov.pr!;
  const sessions = useTerminalsStore((s) => s.sessions);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setCenterTab = useUIStore((s) => s.setTab);
  const showToast = useToastStore((s) => s.show);
  const dismiss = usePrStore((s) => s.dismiss);
  const refresh = usePrStore((s) => s.refresh);
  const [busy, setBusy] = useState(false);
  const [wording, setWording] = useState(false);
  const [reason, setReason] = useState("");
  const gate = mergeGate(ov);
  const termId = ov.evidence.commits.find((c) => c.termId)?.termId;
  const session = sessions.find((s) => s.id === termId);
  const failed = failedChecks(ov);

  const tier: { tier: DecisionTier; label: string; reason: string } =
    why === "ci_red"
      ? {
          tier: "medium",
          label: "CI red",
          reason: `${failed.join(", ") || "a check"} failed on ${pr.headSha.slice(0, 7)}`,
        }
      : why === "comments"
        ? {
            tier: "low",
            label: "Review comments",
            reason: `${pr.comments.length} comment${pr.comments.length === 1 ? "" : "s"} not yet handed to the agent`,
          }
        : gate.gate === "verified"
          ? {
              tier: "low",
              label: "Ready",
              reason: "CI green and every turn behind the branch verified",
            }
          : {
              tier: "medium",
              label: "Ready, on your word",
              reason: `CI green but ${gate.reasons.join(", ")}`,
            };

  const autofix = async () => {
    setBusy(true);
    try {
      if (session) {
        setActive(session.id);
        goTo("sessions");
        setCenterTab("terminal");
      }
      const sent = await typeIntoActiveSession(autofixPrompt(ov));
      await ipc.prAutofix(
        pr.number,
        pr.comments.map((c) => c.id),
      );
      showToast(
        sent
          ? "Review comments typed into the agent — read, then send"
          : "Prompt copied: the session is not live, paste it when it is",
        sent ? "success" : "info",
      );
      await refresh();
    } catch (e) {
      showToast(`Couldn't hand the comments over: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const merge = async (how: "verified" | "human") => {
    setBusy(true);
    try {
      const out = await ipc.prMerge(pr.number, how, how === "human" ? reason.trim() : undefined);
      showToast(
        pr.ci === "green"
          ? `Merged #${pr.number}`
          : `#${pr.number} will merge when green${out ? ` — ${out.split("\n")[0]}` : ""}`,
        "success",
      );
      setWording(false);
      await refresh();
    } catch (e) {
      showToast(`Merge not recorded: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const open = () => void openUrl(pr.url).catch((e) => showToast(String(e), "error"));

  const actions = wording
    ? [
        { label: "Back", variant: "ghost" as const, onClick: () => setWording(false) },
        {
          label: pr.ci === "green" ? "Merge on my word" : "Merge when green, on my word",
          variant: "danger" as const,
          disabled: reason.trim().length < 8,
          onClick: () => void merge("human"),
        },
      ]
    : [
        {
          label: "Later",
          variant: "ghost" as const,
          onClick: () => dismiss(prKey(ov) ?? ""),
        },
        { label: "Open PR", variant: "secondary" as const, onClick: open },
        ...(pr.comments.length > 0
          ? [{ label: "Auto-fix", variant: "secondary" as const, onClick: () => void autofix() }]
          : []),
        gate.gate === "verified"
          ? {
              label: pr.ci === "green" ? "Merge" : "Merge when green",
              variant: "primary" as const,
              onClick: () => void merge("verified"),
            }
          : {
              label: "Merge needs your word",
              variant: "secondary" as const,
              onClick: () => setWording(true),
            },
      ];

  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`pr-${pr.number}`}
        tier={tier.tier}
        tierLabel={tier.label}
        tierReason={tier.reason}
        busy={busy}
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip tone={ciTone(pr.ci as PrCi)} mono data-testid="pr-ci">
              {ciLabel(pr.ci)}
            </Chip>
            <Chip tone={gate.gate === "verified" ? "ok" : "warn"} mono data-testid="pr-evidence">
              {evidenceLabel(ov)}
            </Chip>
            <span>
              PR #{pr.number} · {pr.title}
            </span>
          </span>
        }
        context={`${pr.headRef} → ${pr.baseRef} · ${session?.name ?? (termId ? termId : "no session bound")} · ${pr.comments.length} comment${pr.comments.length === 1 ? "" : "s"} · ${pr.checks.length} check${pr.checks.length === 1 ? "" : "s"}`}
        preview={
          <div className="flex flex-col gap-1.5 px-2 py-1.5 font-sans text-small text-foreground">
            {pr.comments.slice(0, 3).map((c) => (
              <p key={c.id} className="m-0 truncate" title={c.body}>
                <span className="font-code text-caption text-faint-foreground">
                  {c.path ? `${c.path}${c.line ? `:${c.line}` : ""}` : c.kind}
                </span>{" "}
                <span className="text-muted-foreground">@{c.author}:</span>{" "}
                {c.body
                  .replace(/<!--[\s\S]*?-->/g, "")
                  .replace(/\s+/g, " ")
                  .trim()}
              </p>
            ))}
            {gate.reasons.length > 0 && (
              <p className="m-0 text-muted-foreground" data-testid="pr-gate">
                Evidence holds the automatic merge: {gate.reasons.join(", ")}.
              </p>
            )}
            {wording && (
              <Textarea
                aria-label="Why you merge without every turn verified"
                placeholder="Why merging is right without the evidence (goes into the ledger)"
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
