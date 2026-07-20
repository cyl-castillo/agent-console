import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { useAgentStatusStore } from "./agentStatusStore";
import { useToastStore } from "./toastStore";
import { notify, windowIsFocused } from "../lib/notify";
import type { ApprovalRequest } from "../types/domain";

interface ApprovalState {
  queue: ApprovalRequest[];
  /// Resolves true when the decision reached the hook; false when delivery
  /// failed (the request stays queued for retry).
  decide: (id: string, decision: "allow" | "deny" | "ask", reason?: string) => Promise<boolean>;
  _enqueue: (req: ApprovalRequest) => void;
  /// Reconcile the queue against the on-disk pending requests (the source of
  /// truth: the hook deletes each req file once decided or timed out). Heals
  /// a missed `approval://request` event — webview reload, dev HMR, listener
  /// not yet attached — which otherwise leaves the agent stalling invisibly
  /// until the hook's timeout.
  resync: () => Promise<void>;
}

export const useApprovalStore = create<ApprovalState>((set) => ({
  queue: [],

  decide: async (id, decision, reason) => {
    try {
      await ipc.approvalRespond(id, decision, reason);
    } catch (e) {
      // The hook is still polling for this answer. Dropping the request on a
      // failed delivery (the old behavior) made the UI look "answered" while
      // the agent stalled to its timeout — keep it queued and say so.
      console.error("[approvals] respond failed:", e);
      useToastStore
        .getState()
        .show(`Couldn't deliver the approval: ${String(e).slice(0, 100)} — try again`, "error");
      return false;
    }
    set((s) => ({ queue: s.queue.filter((r) => r.id !== id) }));
    // Answering an approval means the agent resumes — keep "working" alive.
    useAgentStatusStore.getState().markActive();
    return true;
  },

  _enqueue: (req) => {
    let added = false;
    set((s) => {
      if (s.queue.some((r) => r.id === req.id)) return s;
      added = true;
      return { queue: [...s.queue, req] };
    });
    useAgentStatusStore.getState().markActive();
    // You're in another window and the agent just blocked on you — the one
    // moment a system notification earns its interruption.
    if (added && !windowIsFocused()) {
      notify("Agent Console — approval needed", `${req.tool} is waiting for your decision`);
    }
  },

  resync: async () => {
    let pending: ApprovalRequest[];
    try {
      pending = await ipc.approvalsPending();
    } catch {
      return;
    }
    const ids = new Set(pending.map((r) => r.id));
    // Drop queued entries whose req file is gone (answered elsewhere or timed
    // out) — but only when older than a grace window, so a request whose
    // event arrived while this fetch was in flight isn't dropped by mistake.
    const cutoff = Date.now() - 5000;
    set((s) => ({
      queue: s.queue.filter((r) => ids.has(r.id) || r.ts > cutoff),
    }));
    for (const req of pending) {
      useApprovalStore.getState()._enqueue(req);
    }
  },
}));

export async function attachApprovalListener(): Promise<UnlistenFn> {
  const un = await listen<ApprovalRequest>("approval://request", (e) => {
    useApprovalStore.getState()._enqueue(e.payload);
  });
  // Event stream + disk resync: events give latency, the resync gives truth.
  // Initial sync covers requests that arrived before this listener existed;
  // the focus sync covers anything missed while the webview was reloading or
  // the user was in another window.
  void useApprovalStore.getState().resync();
  const onFocus = () => void useApprovalStore.getState().resync();
  window.addEventListener("focus", onFocus);
  return () => {
    window.removeEventListener("focus", onFocus);
    un();
  };
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
export function blockedSessionIds(queue: ApprovalRequest[], sessions: SessionRef[]): Set<string> {
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
