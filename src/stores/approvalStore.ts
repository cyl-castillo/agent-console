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
