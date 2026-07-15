import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { ProofEvent, TestigoVerifyReport, TestigoExportSummary } from "../types/domain";

/// One row of the case list: an intent thread with its activity envelope.
export interface CaseSummary {
  caseId: string;
  events: number;
  turns: number;
  approvals: number;
  lastTs: number;
}

interface ProofState {
  projectRoot: string | null;
  events: ProofEvent[];
  report: TestigoVerifyReport | null;
  exporting: string | null;
  lastExport: TestigoExportSummary | null;
  error: string | null;

  load: (projectRoot: string) => Promise<void>;
  clear: () => void;
  exportPack: (caseId?: string) => Promise<void>;
}

export function summarizeCases(events: ProofEvent[]): CaseSummary[] {
  const by = new Map<string, CaseSummary>();
  for (const e of events) {
    const c = by.get(e.caseId) ?? {
      caseId: e.caseId,
      events: 0,
      turns: 0,
      approvals: 0,
      lastTs: 0,
    };
    c.events += 1;
    if (e.kind === "prompt") c.turns += 1;
    if (e.kind === "approval_decision") c.approvals += 1;
    if (e.ts > c.lastTs) c.lastTs = e.ts;
    by.set(e.caseId, c);
  }
  // Most recently active first.
  return [...by.values()].sort((a, b) => b.lastTs - a.lastTs);
}

export const useProofStore = create<ProofState>((set, get) => ({
  projectRoot: null,
  events: [],
  report: null,
  exporting: null,
  lastExport: null,
  error: null,

  load: async (projectRoot) => {
    set({ projectRoot, error: null });
    try {
      const [events, report] = await Promise.all([
        ipc.testigoList(projectRoot),
        ipc.testigoVerify(projectRoot),
      ]);
      set({ events, report });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  clear: () =>
    set({ projectRoot: null, events: [], report: null, lastExport: null, error: null }),

  exportPack: async (caseId) => {
    const root = get().projectRoot;
    if (!root) return;
    set({ exporting: caseId ?? "__ledger__", error: null });
    try {
      const lastExport = await ipc.testigoExport(root, caseId);
      set({ lastExport, exporting: null });
    } catch (e) {
      set({ exporting: null, error: String(e) });
    }
  },
}));
