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
  /// F10: sessionId → when the work moved to OpenCode at the cap. The breach
  /// stays (so the watcher does not fire again on the idle session); the
  /// Inbox hides it. Cleared when the cap is raised above the breach.
  handedOff: Record<string, number>;
  /// F10: turn ids whose quota card the human set aside ("wait for the window").
  quotaDismissed: string[];
  setCap: (sessionId: string, usd: number) => void;
  setDefaultCap: (usd: number) => void;
  markBreached: (sessionId: string, costUsd: number) => void;
  clearBreach: (sessionId: string) => void;
  markHandedOff: (sessionId: string) => void;
  dismissQuota: (turnId: string) => void;
}

const KEY = "agent-console:budgets";

type Persisted = Pick<
  BudgetState,
  "caps" | "defaultCap" | "breached" | "handedOff" | "quotaDismissed"
>;
const EMPTY: Persisted = {
  caps: {},
  defaultCap: 0,
  breached: {},
  handedOff: {},
  quotaDismissed: [],
};

function load(): Persisted {
  try {
    if (typeof localStorage === "undefined") return { ...EMPTY };
    const raw = localStorage.getItem(KEY);
    const p = raw ? (JSON.parse(raw) as Partial<BudgetState>) : {};
    return {
      caps: p.caps ?? {},
      defaultCap: p.defaultCap ?? 0,
      breached: p.breached ?? {},
      handedOff: p.handedOff ?? {},
      quotaDismissed: p.quotaDismissed ?? [],
    };
  } catch {
    return { ...EMPTY };
  }
}

function persist(s: BudgetState) {
  try {
    localStorage.setItem(
      KEY,
      JSON.stringify({
        caps: s.caps,
        defaultCap: s.defaultCap,
        breached: s.breached,
        handedOff: s.handedOff,
        quotaDismissed: s.quotaDismissed,
      }),
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
    const handedOff = { ...get().handedOff };
    if (usd <= 0 || (breached[sessionId] ?? 0) < usd) {
      delete breached[sessionId];
      // A raised cap reopens the session: the hand-over mark goes with it.
      delete handedOff[sessionId];
    }
    set({ caps, breached, handedOff });
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
    const handedOff = { ...get().handedOff };
    delete breached[sessionId];
    delete handedOff[sessionId];
    set({ breached, handedOff });
    persist(get());
  },
  markHandedOff: (sessionId) => {
    set({ handedOff: { ...get().handedOff, [sessionId]: Date.now() } });
    persist(get());
  },
  dismissQuota: (turnId) => {
    const cur = get().quotaDismissed;
    if (cur.includes(turnId)) return;
    // Bounded: the newest 50 are enough; older turns no longer surface.
    set({ quotaDismissed: [...cur, turnId].slice(-50) });
    persist(get());
  },
}));

/// The cap that applies to a session: its own, else the project default.
export function capFor(s: Pick<BudgetState, "caps" | "defaultCap">, sessionId: string): number {
  return s.caps[sessionId] ?? s.defaultCap;
}
