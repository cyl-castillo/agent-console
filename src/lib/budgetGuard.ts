import { ipc } from "../ipc/tauri";
import { capFor, useBudgetStore } from "../stores/budgetStore";
import type { TerminalSession } from "../stores/terminalsStore";

export type BudgetDecisionKind = "stop" | "summary" | "raise" | "switch";

interface BudgetView {
  caps: Record<string, number>;
  defaultCap: number;
  breached: Record<string, number>;
}

/// The sessions whose reported cost just crossed their cap and are not yet
/// marked breached: live, capped, cost known and ≥ cap. Pure, so the hard
/// stop is testable without a PTY.
export function findBreaches(
  byTerm: Record<string, { costUsd?: number } | undefined>,
  budgets: BudgetView,
  sessions: TerminalSession[],
): { session: TerminalSession; spent: number; cap: number }[] {
  const out: { session: TerminalSession; spent: number; cap: number }[] = [];
  for (const s of sessions) {
    if (s.status !== "live") continue;
    const cap = capFor(budgets, s.id);
    if (cap <= 0) continue;
    const cost = byTerm[s.id]?.costUsd;
    if (cost === undefined || cost < cap) continue;
    if (budgets.breached[s.id] !== undefined) continue;
    out.push({ session: s, spent: cost, cap });
  }
  return out;
}

/// D5: the pause and its answer as ledger evidence. `budget_reached` when the
/// console interrupts at the cap; `budget_decision` when the human answers.
/// Witnessing can be off for the project — then the ledger refuses and the
/// stop still happens; the evidence is best-effort, the interrupt is not.
export async function recordBudget(
  projectRoot: string | null | undefined,
  session: Pick<TerminalSession, "id" | "agentSessionId">,
  spent: number,
  cap: number,
  decision?: BudgetDecisionKind,
  newCap?: number,
): Promise<boolean> {
  if (!projectRoot) return false;
  try {
    await ipc.testigoBudgetEvent(
      projectRoot,
      session.id,
      session.agentSessionId ?? undefined,
      spent,
      cap,
      decision,
      newCap,
    );
    return true;
  } catch {
    return false;
  }
}

/// The cap currently applying to a session (own cap or project default).
export function currentCap(sessionId: string): number {
  return capFor(useBudgetStore.getState(), sessionId);
}
