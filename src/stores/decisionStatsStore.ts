import { create } from "zustand";
import type { DecisionTier } from "../lib/decisions";

/// The habituation metric (Pan, 2026): approval rate per tier over a rolling
/// window. A gate that approves ≥ 95 % of what it sees has decayed into a
/// reflex — the Inbox says so instead of letting it happen silently.
export interface DecisionRecord {
  ts: number;
  tier: DecisionTier;
  decision: "allow" | "deny";
}

const KEY = "agent-console:decision-stats";
export const WINDOW_MS = 14 * 86_400_000;
export const DECAYED_RATE = 0.95;
export const MIN_SAMPLE = 10;

interface StatsState {
  records: DecisionRecord[];
  record: (tier: DecisionTier, decision: "allow" | "deny") => void;
}

function load(): DecisionRecord[] {
  try {
    if (typeof localStorage === "undefined") return [];
    const raw = localStorage.getItem(KEY);
    const cutoff = Date.now() - WINDOW_MS;
    return raw ? (JSON.parse(raw) as DecisionRecord[]).filter((r) => r.ts >= cutoff) : [];
  } catch {
    return [];
  }
}

export const useDecisionStatsStore = create<StatsState>((set, get) => ({
  records: load(),
  record: (tier, decision) => {
    const cutoff = Date.now() - WINDOW_MS;
    const records = [
      ...get().records.filter((r) => r.ts >= cutoff),
      { ts: Date.now(), tier, decision },
    ];
    set({ records });
    try {
      localStorage.setItem(KEY, JSON.stringify(records));
    } catch {
      /* ignore */
    }
  },
}));

export interface TierRate {
  tier: DecisionTier;
  total: number;
  rate: number | null;
  decayed: boolean;
}

export function tierRates(records: DecisionRecord[], now = Date.now()): TierRate[] {
  const cutoff = now - WINDOW_MS;
  return (["low", "medium", "high"] as DecisionTier[]).map((tier) => {
    const rs = records.filter((r) => r.tier === tier && r.ts >= cutoff);
    const total = rs.length;
    const rate = total ? rs.filter((r) => r.decision === "allow").length / total : null;
    return {
      tier,
      total,
      rate,
      decayed: total >= MIN_SAMPLE && rate !== null && rate >= DECAYED_RATE,
    };
  });
}
