import { useApprovalStore } from "../stores/approvalStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { useToastStore } from "../stores/toastStore";
import { useDecisionStatsStore } from "../stores/decisionStatsStore";
import { codexEquivalent } from "../permissions/rules";
import type { RuleSuggestion } from "../permissions/types";
import type { ApprovalRequest } from "../types/domain";
import { tierFor } from "./decisions";

/// Decide a pending approval and record it for the habituation metric.
/// Returns whether the decision reached the hook.
export async function decideApproval(
  req: ApprovalRequest,
  decision: "allow" | "deny",
  reason: string,
): Promise<boolean> {
  const delivered = await useApprovalStore.getState().decide(req.id, decision, reason);
  if (delivered) useDecisionStatsStore.getState().record(tierFor(req).tier, decision);
  return delivered;
}

/// "Always…": persist the rule in the right engine's store, then answer the
/// pending request the same way. A Codex approval with a translatable rule
/// goes to Codex's execpolicy store; otherwise the rule still lands in
/// Claude's settings.json (the panel already warned it won't bind there).
export async function commitAlways(
  req: ApprovalRequest,
  engine: "claude" | "codex",
  suggestion: RuleSuggestion,
  effect: "allow" | "deny",
): Promise<boolean> {
  const ruleEngine =
    engine === "codex" && codexEquivalent(suggestion.rule.raw) ? "codex" : "claude";
  const saved = await usePermissionsStore
    .getState()
    .add(suggestion.rule.scope, effect, suggestion.rule.raw, ruleEngine);
  if (!saved) {
    // Surface WHY the rule didn't save — a silent stay-open card reads as
    // "nothing happened" and the approval quietly expires.
    const err = usePermissionsStore.getState().error;
    useToastStore.getState().show(`Rule not saved${err ? `: ${err}` : ""}`, "error");
    return false;
  }
  return decideApproval(req, effect, `rule saved: ${suggestion.rule.raw}`);
}
