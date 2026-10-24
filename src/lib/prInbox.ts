import type { PrComment, PrOverview, ProofEvent } from "../types/domain";

/// F7: what the PR card says and when it is a decision at all. Pure, so the
/// vectors are tests; the card and the session row read the same words.

/// Comments carried into the auto-fix prompt, and how much of each.
export const AUTOFIX_COMMENTS_CAP = 20;
export const AUTOFIX_BODY_CAP = 400;

export type PrCi = "none" | "pending" | "green" | "red";

export function ciLabel(ci: string): string {
  switch (ci) {
    case "green":
      return "CI green";
    case "red":
      return "CI red";
    case "pending":
      return "CI running";
    default:
      return "no CI";
  }
}

export function ciTone(ci: string): "ok" | "warn" | "danger" | "neutral" {
  switch (ci) {
    case "green":
      return "ok";
    case "red":
      return "danger";
    case "pending":
      return "warn";
    default:
      return "neutral";
  }
}

/// `verified 2/3` — how many of the branch's commits sit on a verified (or
/// human-accepted) turn.
export function evidenceLabel(ov: PrOverview): string {
  const e = ov.evidence;
  return `verified ${e.verified + e.accepted}/${e.commits.length}`;
}

/// The merge gate as the card states it: by evidence, or on your word.
export function mergeGate(ov: PrOverview): {
  gate: "verified" | "human";
  reasons: string[];
} {
  const e = ov.evidence;
  const reasons: string[] = [];
  if (e.commits.length === 0) reasons.push("no commits on this branch");
  if (e.unbound > 0)
    reasons.push(`${e.unbound} commit${e.unbound === 1 ? "" : "s"} the ledger never saw`);
  if (e.unverified > 0)
    reasons.push(`${e.unverified} turn${e.unverified === 1 ? "" : "s"} without a verdict`);
  if (e.contradicted > 0)
    reasons.push(`${e.contradicted} claim${e.contradicted === 1 ? "" : "s"} contradicted`);
  return { gate: e.allVerified ? "verified" : "human", reasons };
}

/// Whether a `pr_autofix` line already names this head: the comments were
/// handed to the agent once; a new push resets it.
export function autofixedAt(events: ProofEvent[], number: number, headSha: string): boolean {
  return events.some(
    (e) => e.kind === "pr_autofix" && e.payload.number === number && e.payload.headSha === headSha,
  );
}

/// The PR is a decision when: CI is red (fix or say so), the reviewers left
/// comments not yet handed to the agent, or it is green and open (merge is a
/// word, not a reflex). Running CI without comments is a row chip, not a card.
export function prNeedsYou(
  ov: PrOverview | null,
  events: ProofEvent[],
  dismissed: string[] = [],
): "ci_red" | "comments" | "ready" | null {
  const pr = ov?.pr;
  if (!ov || !pr || pr.state !== "OPEN" || pr.isDraft) return null;
  if (dismissed.includes(`${pr.number}:${pr.headSha}`)) return null;
  const fresh = pr.comments.length > 0 && !autofixedAt(events, pr.number, pr.headSha);
  if (pr.ci === "red") return "ci_red";
  if (fresh) return "comments";
  if (pr.ci === "green") return "ready";
  return null;
}

function oneLine(s: string): string {
  return s
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/// The prompt "Auto-fix" types into the agent: every review comment with
/// its file and line, then the gate the next verdict reads. The human
/// sends it — nothing here runs on its own, and no bot answers the PR.
export function autofixPrompt(ov: PrOverview): string {
  const pr = ov.pr;
  if (!pr) return "";
  const comments: PrComment[] = pr.comments.slice(0, AUTOFIX_COMMENTS_CAP);
  const lines = comments.map((c, i) => {
    const where = c.path ? `${c.path}${c.line ? `:${c.line}` : ""}` : c.kind;
    let body = oneLine(c.body);
    if (body.length > AUTOFIX_BODY_CAP) body = `${body.slice(0, AUTOFIX_BODY_CAP)}…`;
    return `${i + 1}. ${where} — @${c.author}: ${body}`;
  });
  const more =
    pr.comments.length > comments.length
      ? `\n(${pr.comments.length - comments.length} more on the PR page.)`
      : "";
  const ci = pr.ci === "red" ? `CI is red (${failedChecks(ov).join(", ") || "see checks"}). ` : "";
  return `${ci}Address the review comments on PR #${pr.number} (${pr.url}). For each one, fix it or say in one line why not. Then run the project's test gate and report the exit code and the runner's summary line — do not say it passes unless the run exits 0.\n${lines.join("\n")}${more}`;
}

export function failedChecks(ov: PrOverview): string[] {
  return (ov.pr?.checks ?? []).filter((c) => c.status === "failure").map((c) => c.name);
}

/// The sessions the PR's commits came from — what the row chip is for.
export function prTermIds(ov: PrOverview | null): Set<string> {
  const out = new Set<string>();
  for (const c of ov?.evidence.commits ?? []) if (c.termId) out.add(c.termId);
  return out;
}
