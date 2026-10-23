import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { reconcileSwitchedModel } from "../agents/profiles";
import { useTerminalsStore } from "./terminalsStore";

/// What the CLI's own status line reports per render (T3): model, cost,
/// context — the numbers Claude Code computes itself, delivered through our
/// statusLine stand-in as `hook://status`. Replaces guessing: until now the
/// pill re-read the transcript every 5 s and assumed the window size.

export interface LiveStatus {
  ts: number;
  termId?: string;
  sessionId?: string;
  modelId?: string;
  modelName?: string;
  costUsd?: number;
  linesAdded?: number;
  linesRemoved?: number;
  durationMs?: number;
  contextSize?: number;
  usedPct?: number;
  /// input + cache_creation + cache_read of the latest turn (the CLI's own
  /// `used_percentage` formula; output tokens excluded).
  contextUsed?: number;
  outputTokens?: number;
  inputTotal?: number;
  outputTotal?: number;
  exceeds200k?: boolean;
  /// F2: the plan's quota windows as the CLI reports them (claude.ai Pro/Max;
  /// each may be absent), 0–100.
  quota5hPct?: number;
  quota5hResetsAt?: number;
  quota7dPct?: number;
  quota7dResetsAt?: number;
  spendPct?: number;
  spendResetsAt?: number;
  spendUsedUsd?: number;
  spendLimitUsd?: number;
  /// F2: the cost came from the ledger's turn closes (engines without a
  /// status line: OpenCode's plugin), not from a live render.
  fromLedger?: boolean;
}

/// A render older than this is stale: the CLI re-renders on every assistant
/// message, so silence this long means the session is idle or gone, and the
/// transcript poll takes over.
export const LIVE_STATUS_FRESH_MS = 120_000;

interface LiveStatusState {
  byTerm: Record<string, LiveStatus>;
  note: (s: LiveStatus) => void;
  /// F2: a session's cumulative cost as summed from its ledger turn closes
  /// (OpenCode reports per turn through the plugin; Codex reports no
  /// dollars). Never overrides a live status-line render.
  noteLedgerCost: (termId: string, costUsd: number, ts: number) => void;
}

export const useLiveStatusStore = create<LiveStatusState>((set) => ({
  byTerm: {},
  note: (s) => {
    if (!s.termId) return;
    set((st) => ({ byTerm: { ...st.byTerm, [s.termId as string]: s } }));
  },
  noteLedgerCost: (termId, costUsd, ts) => {
    set((st) => {
      const cur = st.byTerm[termId];
      if (cur && !cur.fromLedger) return st;
      if (cur && cur.costUsd === costUsd) return st;
      return { byTerm: { ...st.byTerm, [termId]: { ts, termId, costUsd, fromLedger: true } } };
    });
  },
}));

/// The latest render for a terminal, or null when there is none or it went
/// stale.
export function freshStatus(
  byTerm: Record<string, LiveStatus>,
  termId: string,
  now: number,
): LiveStatus | null {
  const s = byTerm[termId];
  if (!s) return null;
  return now - s.ts <= LIVE_STATUS_FRESH_MS ? s : null;
}

export async function attachLiveStatusListener(): Promise<UnlistenFn> {
  return listen<LiveStatus>("hook://status", (e) => {
    const s = e.payload;
    if (!s?.termId) return;
    useLiveStatusStore.getState().note(s);
    // The status line names the model that is REALLY running, on every
    // render — including session start and resume, where PostModelSwitch
    // never fires. Same reconciliation rule, same termId-only binding.
    if (s.modelId) {
      const session = useTerminalsStore.getState().sessions.find((t) => t.id === s.termId);
      if (session) {
        const next = reconcileSwitchedModel(session.agent, session.model, s.modelId);
        if (next !== null) useTerminalsStore.getState().setModel(session.id, next);
      }
    }
  });
}
