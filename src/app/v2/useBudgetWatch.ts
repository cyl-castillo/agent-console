import { useEffect } from "react";
import { capFor, useBudgetStore } from "../../stores/budgetStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useToastStore } from "../../stores/toastStore";
import type { TermInputDetail } from "../../components/Terminal";

/// The hard stop (D5): when a session's reported cost crosses its cap, send
/// one interrupt to its PTY and mark the breach. The Inbox turns the breach
/// into a decision; nothing here retries or escalates on its own.
export function useBudgetWatch(): void {
  useEffect(
    () =>
      useLiveStatusStore.subscribe((state) => {
        const budgets = useBudgetStore.getState();
        const sessions = useTerminalsStore.getState().sessions;
        for (const s of sessions) {
          if (s.status !== "live") continue;
          const cap = capFor(budgets, s.id);
          if (cap <= 0) continue;
          const cost = state.byTerm[s.id]?.costUsd;
          if (cost === undefined || cost < cap) continue;
          if (budgets.breached[s.id] !== undefined) continue;
          budgets.markBreached(s.id, cost);
          const detail: TermInputDetail = { sessionId: s.id, data: "\x1b" };
          window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
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
