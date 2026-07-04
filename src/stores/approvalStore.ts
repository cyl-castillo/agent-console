import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { useAgentStatusStore } from "./agentStatusStore";
import type { ApprovalRequest } from "../types/domain";

interface ApprovalState {
  queue: ApprovalRequest[];
  decide: (id: string, decision: "allow" | "deny" | "ask", reason?: string) => Promise<void>;
  _enqueue: (req: ApprovalRequest) => void;
}

export const useApprovalStore = create<ApprovalState>((set) => ({
  queue: [],

  decide: async (id, decision, reason) => {
    try { await ipc.approvalRespond(id, decision, reason); } catch { /* ignore */ }
    set((s) => ({ queue: s.queue.filter((r) => r.id !== id) }));
    // Answering an approval means the agent resumes — keep "working" alive.
    useAgentStatusStore.getState().markActive();
  },

  _enqueue: (req) => {
    set((s) => {
      if (s.queue.some((r) => r.id === req.id)) return s;
      return { queue: [...s.queue, req] };
    });
    useAgentStatusStore.getState().markActive();
  },
}));

export async function attachApprovalListener(): Promise<UnlistenFn> {
  return await listen<ApprovalRequest>("approval://request", (e) => {
    useApprovalStore.getState()._enqueue(e.payload);
  });
}

/// Minimal session shape the attribution needs — structural, so this store
/// doesn't import terminalsStore (no store-to-store coupling).
export interface SessionRef {
  id: string;
  cwd: string;
  status: string;
}

/// Which sessions are blocked waiting on a queued approval.
///
/// Attribution, strongest first:
/// 1. `termId` — the hook tags each request with the PTY's terminal-session id
///    (AGENT_CONSOLE_TERM_ID), which IS the session id. Deterministic.
/// 2. cwd — for requests without a termId (agents launched before the hook
///    captured it): only when exactly ONE live session runs in that cwd.
///    Ambiguous matches attribute to nobody — a wrong "waiting" badge on the
///    wrong session is worse than none (the global StatusBar pill still covers
///    the queue as a whole).
export function blockedSessionIds(
  queue: ApprovalRequest[],
  sessions: SessionRef[],
): Set<string> {
  const blocked = new Set<string>();
  for (const req of queue) {
    if (req.termId && sessions.some((s) => s.id === req.termId)) {
      blocked.add(req.termId);
      continue;
    }
    const inCwd = sessions.filter((s) => s.status === "live" && s.cwd === req.cwd);
    if (inCwd.length === 1) blocked.add(inCwd[0].id);
  }
  return blocked;
}
