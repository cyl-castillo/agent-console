import { create } from "zustand";

/// Hard budgets per session (D5), in USD, kept locally per project. The CLI's
/// own status line reports the running cost (hook://status); when it crosses
/// the budget the console interrupts the turn once and puts a decision in
/// the Inbox — raise, stop, or ask for a summary. Never silent.
interface BudgetState {
  /// sessionId → USD cap.
  caps: Record<string, number>;
  /// Default cap applied to new sessions of this project (0 = none).
  defaultCap: number;
  /// sessionId → cost at which the cap was crossed and the turn interrupted;
  /// cleared when the cap is raised above it.
  breached: Record<string, number>;
  setCap: (sessionId: string, usd: number) => void;
  setDefaultCap: (usd: number) => void;
  markBreached: (sessionId: string, costUsd: number) => void;
  clearBreach: (sessionId: string) => void;
}

const KEY = "agent-console:budgets";

function load(): Pick<BudgetState, "caps" | "defaultCap" | "breached"> {
  try {
    if (typeof localStorage === "undefined") return { caps: {}, defaultCap: 0, breached: {} };
    const raw = localStorage.getItem(KEY);
    const p = raw ? (JSON.parse(raw) as Partial<BudgetState>) : {};
    return { caps: p.caps ?? {}, defaultCap: p.defaultCap ?? 0, breached: p.breached ?? {} };
  } catch {
    return { caps: {}, defaultCap: 0, breached: {} };
  }
}

function persist(s: BudgetState) {
  try {
    localStorage.setItem(
      KEY,
      JSON.stringify({ caps: s.caps, defaultCap: s.defaultCap, breached: s.breached }),
    );
  } catch {
    /* ignore */
  }
}

export const useBudgetStore = create<BudgetState>((set, get) => ({
  ...load(),
  setCap: (sessionId, usd) => {
    const caps = { ...get().caps };
    if (usd > 0) caps[sessionId] = usd;
    else delete caps[sessionId];
    const breached = { ...get().breached };
    if (usd <= 0 || (breached[sessionId] ?? 0) < usd) delete breached[sessionId];
    set({ caps, breached });
    persist(get());
  },
  setDefaultCap: (defaultCap) => {
    set({ defaultCap: Math.max(0, defaultCap) });
    persist(get());
  },
  markBreached: (sessionId, costUsd) => {
    set({ breached: { ...get().breached, [sessionId]: costUsd } });
    persist(get());
  },
  clearBreach: (sessionId) => {
    const breached = { ...get().breached };
    delete breached[sessionId];
    set({ breached });
    persist(get());
  },
}));

/// The cap that applies to a session: its own, else the project default.
export function capFor(s: Pick<BudgetState, "caps" | "defaultCap">, sessionId: string): number {
  return s.caps[sessionId] ?? s.defaultCap;
}
