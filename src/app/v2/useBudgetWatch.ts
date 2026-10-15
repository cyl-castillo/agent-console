import { useEffect } from "react";
import { useBudgetStore } from "../../stores/budgetStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useToastStore } from "../../stores/toastStore";
import { findBreaches, recordBudget } from "../../lib/budgetGuard";
import type { TermInputDetail } from "../../components/Terminal";

/// The hard stop (D5): when a session's reported cost crosses its cap, send
/// one interrupt to its PTY, mark the breach and witness `budget_reached` in
/// the ledger. The Inbox turns the breach into a decision; nothing here
/// retries or escalates on its own.
export function useBudgetWatch(): void {
  useEffect(
    () =>
      useLiveStatusStore.subscribe((state) => {
        const budgets = useBudgetStore.getState();
        const sessions = useTerminalsStore.getState().sessions;
        const root = useSessionStore.getState().project?.root;
        for (const { session: s, spent, cap } of findBreaches(state.byTerm, budgets, sessions)) {
          budgets.markBreached(s.id, spent);
          const detail: TermInputDetail = { sessionId: s.id, data: "\x1b" };
          window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
          void recordBudget(root, s, spent, cap);
          useToastStore
            .getState()
            .show(
              `${s.name} reached its $${cap.toFixed(2)} budget — turn interrupted, decide in the Inbox`,
              "error",
            );
        }
      }),
    [],
  );
}
