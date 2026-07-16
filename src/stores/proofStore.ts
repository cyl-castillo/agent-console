import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type {
  ProofEvent,
  TestigoVerifyReport,
  TestigoExportSummary,
  TestigoExportPreview,
} from "../types/domain";

/// One row of the case list: an intent thread with its activity envelope.
export interface CaseSummary {
  caseId: string;
  events: number;
  turns: number;
  approvals: number;
  lastTs: number;
}

/// One turn of a case timeline: intent → approvals → results, assembled from
/// the flat event stream.
export interface TimelineTurn {
  turnId: string | null;
  ts: number;
  prompt: string;
  skill?: string;
  approvals: { tool?: string; decision?: string; reason?: string }[];
  toolResults: number;
  files: { status: string; path: string }[];
  filesTruncated: boolean;
  endTs: number | null;
}

interface ProofState {
  projectRoot: string | null;
  events: ProofEvent[];
  report: TestigoVerifyReport | null;
  exporting: string | null;
  lastExport: TestigoExportSummary | null;
  error: string | null;
  /// Case opened in the timeline view; null = case list.
  selectedCase: string | null;
  /// Pre-sign review in progress (null = none). `undefined` caseId inside it
  /// means "full ledger".
  review: { caseId?: string; preview: TestigoExportPreview; redactSeqs: number[] } | null;

  load: (projectRoot: string) => Promise<void>;
  clear: () => void;
  /// Step 1: open the pre-sign review for a case (or the full ledger).
  startExport: (caseId?: string) => Promise<void>;
  /// Toggle manual redaction of one event in the open review.
  toggleRedact: (seq: number) => void;
  /// Step 2: sign and write the packet with the chosen redactions.
  confirmExport: () => Promise<void>;
  cancelExport: () => void;
  selectCase: (caseId: string | null) => void;
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

/// Fold a case's events into turns (chronological). Events keyed by turnId;
/// a prompt opens the turn, turn_end closes it with the diff payload. Events
/// with no turn (case_link, job_run, post-restart strays) are skipped — the
/// case header already shows totals.
export function buildTimeline(events: ProofEvent[]): TimelineTurn[] {
  const turns: TimelineTurn[] = [];
  const byId = new Map<string, TimelineTurn>();
  const turnFor = (e: ProofEvent): TimelineTurn | null => {
    if (!e.turnId) return null;
    let t = byId.get(e.turnId);
    if (!t) {
      t = {
        turnId: e.turnId,
        ts: e.ts,
        prompt: "",
        approvals: [],
        toolResults: 0,
        files: [],
        filesTruncated: false,
        endTs: null,
      };
      byId.set(e.turnId, t);
      turns.push(t);
    }
    return t;
  };
  for (const e of events) {
    const t = turnFor(e);
    if (!t) continue;
    const p = e.payload as Record<string, unknown>;
    if (e.kind === "prompt") {
      t.ts = e.ts;
      t.prompt = typeof p.prompt === "string" ? p.prompt : "";
      if (typeof p.skill === "string") t.skill = p.skill;
    } else if (e.kind === "approval_decision") {
      t.approvals.push({
        tool: typeof p.tool === "string" ? p.tool : undefined,
        decision: typeof p.decision === "string" ? p.decision : undefined,
        reason: typeof p.reason === "string" ? p.reason : undefined,
      });
    } else if (e.kind === "tool_result") {
      t.toolResults += 1;
    } else if (e.kind === "turn_end") {
      t.endTs = e.ts;
      if (Array.isArray(p.filesChanged)) {
        t.files = (p.filesChanged as { status: string; path: string }[]).filter(
          (f) => typeof f?.path === "string",
        );
      }
      t.filesTruncated = p.filesTruncated === true;
    }
  }
  return turns;
}

export const useProofStore = create<ProofState>((set, get) => ({
  projectRoot: null,
  events: [],
  report: null,
  exporting: null,
  lastExport: null,
  error: null,
  selectedCase: null,
  review: null,

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
    set({
      projectRoot: null,
      events: [],
      report: null,
      lastExport: null,
      error: null,
      selectedCase: null,
      review: null,
    }),

  selectCase: (caseId) => set({ selectedCase: caseId }),

  startExport: async (caseId) => {
    const root = get().projectRoot;
    if (!root) return;
    set({ error: null });
    try {
      const preview = await ipc.testigoExportPreview(root, caseId);
      set({ review: { caseId, preview, redactSeqs: [] } });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  toggleRedact: (seq) =>
    set((s) => {
      if (!s.review) return s;
      const has = s.review.redactSeqs.includes(seq);
      return {
        review: {
          ...s.review,
          redactSeqs: has
            ? s.review.redactSeqs.filter((x) => x !== seq)
            : [...s.review.redactSeqs, seq],
        },
      };
    }),

  confirmExport: async () => {
    const { projectRoot: root, review } = get();
    if (!root || !review) return;
    set({ exporting: review.caseId ?? "__ledger__", error: null });
    try {
      const lastExport = await ipc.testigoExport(
        root,
        review.caseId,
        undefined,
        review.redactSeqs,
      );
      set({ lastExport, exporting: null, review: null });
    } catch (e) {
      set({ exporting: null, error: String(e) });
    }
  },

  cancelExport: () => set({ review: null }),
}));
