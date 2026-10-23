import type { TimelineTurn } from "../stores/proofStore";

/// F2: what the agents cost, from the ledger's own turn closes — the number
/// no console shows next to the evidence ("what a week of agent work cost").
export interface UsageWeek {
  turns: number;
  /// Dollars the engines reported (Claude's deltas, OpenCode's sums). Codex
  /// reports tokens only, so its turns count here but not in dollars.
  costUsd: number;
  byEngine: Record<string, { turns: number; costUsd: number; tokensOut: number }>;
  /// Turns whose engine reported no dollars (Codex): tokens only.
  unpricedTurns: number;
}

export function usageWeek(
  turns: TimelineTurn[],
  now = Date.now(),
  windowMs = 7 * 86_400_000,
): UsageWeek {
  const out: UsageWeek = { turns: 0, costUsd: 0, byEngine: {}, unpricedTurns: 0 };
  for (const t of turns) {
    if (!t.usage || now - t.ts > windowMs) continue;
    out.turns++;
    const e = (out.byEngine[t.usage.engine] ??= { turns: 0, costUsd: 0, tokensOut: 0 });
    e.turns++;
    e.tokensOut += t.usage.tokensOut ?? 0;
    if (t.usage.costTurnUsd !== undefined) {
      e.costUsd += t.usage.costTurnUsd;
      out.costUsd += t.usage.costTurnUsd;
    } else out.unpricedTurns++;
  }
  out.costUsd = Math.round(out.costUsd * 100) / 100;
  for (const e of Object.values(out.byEngine)) e.costUsd = Math.round(e.costUsd * 100) / 100;
  return out;
}

/// A session's cumulative cost from its ledger closes — for engines without
/// a status line (OpenCode sums per turn; Claude's deltas add up the same).
export function sessionLedgerCost(turns: TimelineTurn[], termId: string): number | undefined {
  let sum = 0;
  let any = false;
  for (const t of turns) {
    if (t.termId !== termId || t.usage?.costTurnUsd === undefined) continue;
    sum += t.usage.costTurnUsd;
    any = true;
  }
  return any ? Math.round(sum * 1e6) / 1e6 : undefined;
}

/// Short money: $0.004 → "<$0.01", 1.2345 → "$1.23", 12.3 → "$12.30".
export function fmtUsd(v: number): string {
  if (v > 0 && v < 0.01) return "<$0.01";
  return `$${v.toFixed(2)}`;
}
