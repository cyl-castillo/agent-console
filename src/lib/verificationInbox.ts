import type { TimelineTurn } from "../stores/proofStore";

/// F1d: which closed turns still wait for a human word on their verification
/// verdict. A turn needs one when the console found a claim it could not
/// verify (unverified, stale) or that the evidence contradicted — and nobody
/// has answered yet: no `verification_decision`, and no reviewer verdict
/// after it (a review takes the decision over).
export const NEEDS_DECISION = new Set(["unverified_claim", "claim_contradicted", "stale_check"]);

export function pendingVerifications(turns: TimelineTurn[]): TimelineTurn[] {
  return turns.filter(
    (t) =>
      t.verification !== undefined &&
      NEEDS_DECISION.has(t.verification.status) &&
      t.verification.decision === undefined &&
      t.review === undefined,
  );
}

/// The prompt "Ask for evidence" types into the agent: the project's own
/// gate when known, else the generic ask. It names the exit code because
/// that is what the next verdict reads.
export function evidencePrompt(turn: TimelineTurn): string {
  const v = turn.verification;
  const notRun = v ? Object.entries(v.gates).filter(([, s]) => s === "not_run") : [];
  const picked =
    notRun.find(([k]) => k === "test") ?? notRun[0] ?? (v?.gateCommands.test ? ["test"] : null);
  const cmd = picked ? v?.gateCommands[picked[0]] : undefined;
  if (cmd) {
    return `Run \`${cmd}\` now and report the exit code and the runner's summary line. Do not say the tests pass unless this run exits 0.`;
  }
  return "Run the project's test suite now and report the exit code and the runner's summary line. Do not say the tests pass unless this run exits 0.";
}

/// Last-7-days habit metric for the Proof header: how often a claim was
/// backed by a passing check after the last edit.
export function verificationStats(
  turns: TimelineTurn[],
  now = Date.now(),
  windowMs = 7 * 86_400_000,
) {
  const recent = turns.filter((t) => t.verification && now - t.ts <= windowMs);
  const by = (s: string) => recent.filter((t) => t.verification?.status === s).length;
  const verified = by("verified");
  const unverified = by("unverified_claim") + by("stale_check");
  const contradicted = by("claim_contradicted");
  const claims = verified + unverified + contradicted;
  return { turns: recent.length, claims, verified, unverified, contradicted };
}
