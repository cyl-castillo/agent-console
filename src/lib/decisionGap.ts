import type { DecisionGap } from "../types/domain";

/// "2 s", "90 s", "7 min", "31 h" — the wait a decision had, coarse on purpose.
export function waitLabel(ms: number): string {
  if (ms < 60_000) return `${Math.max(1, Math.round(ms / 1000))} s`;
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)} min`;
  return `${Math.round(ms / 3_600_000)} h`;
}

/// MOVIL F0: the sentence the gap makes, or null when the ledger has no
/// requests in the window. Numbers are the ledger's, never an estimate.
export function gapSentence(gap: DecisionGap | undefined): string | null {
  if (!gap || gap.requests === 0) return null;
  const week = `Last ${gap.windowDays} days`;
  const waits =
    gap.p90WaitMs !== undefined && gap.maxWaitMs !== undefined
      ? ` The ones taken here waited ${waitLabel(gap.p90WaitMs)} at the 90th percentile and ${waitLabel(gap.maxWaitMs)} at most.`
      : "";
  if (gap.unseen === 0) {
    return `${week}: all ${gap.requests} decisions were answered here.${waits}`;
  }
  return `${week}: ${gap.unseen} of ${gap.requests} decisions timed out or were answered in the CLI before this screen saw them.${waits} A paired device catches them wherever you are.`;
}
