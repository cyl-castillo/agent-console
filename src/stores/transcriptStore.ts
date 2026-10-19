import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// F5: what the agent writes while it works, straight from the CLI's own
/// transcript (Claude's `<session>.jsonl`, Codex's rollout). Presentation
/// only — the ledger stays the truth about what a turn did.
export type TranscriptItem =
  | { kind: "text"; id: string; ts: number; text: string }
  | { kind: "tool_use"; id: string; ts: number; name: string; input: string }
  | { kind: "tool_result"; id: string; ts: number; ok: boolean }
  | { kind: "user"; id: string; ts: number; text: string };

export interface TranscriptDelta {
  sessionId: string;
  engine: string;
  backfill: boolean;
  items: TranscriptItem[];
}

/// Per agent session, the most recent items; older ones belong to turns the
/// ledger already closed, so the cap only trims what nobody reads.
export const TRANSCRIPT_CAP = 600;
const EMPTY: TranscriptItem[] = [];

interface TranscriptState {
  bySession: Record<string, TranscriptItem[]>;
  apply: (delta: TranscriptDelta) => void;
  clear: (sessionId: string) => void;
}

export const useTranscriptStore = create<TranscriptState>((set) => ({
  bySession: {},
  apply: (delta) =>
    set((s) => {
      const prev = delta.backfill ? EMPTY : (s.bySession[delta.sessionId] ?? EMPTY);
      const seen = new Set(prev.map((i) => `${i.kind}:${i.id}`));
      const fresh = delta.items.filter((i) => !seen.has(`${i.kind}:${i.id}`));
      if (fresh.length === 0 && !delta.backfill) return s;
      const next = [...prev, ...fresh].slice(-TRANSCRIPT_CAP);
      return { bySession: { ...s.bySession, [delta.sessionId]: next } };
    }),
  clear: (sessionId) =>
    set((s) => {
      if (!(sessionId in s.bySession)) return s;
      const bySession = { ...s.bySession };
      delete bySession[sessionId];
      return { bySession };
    }),
}));

/// Stable selector: the same `EMPTY` for a session with nothing yet.
export const itemsFor = (sessionId: string | undefined) => (s: TranscriptState) =>
  sessionId ? (s.bySession[sessionId] ?? EMPTY) : EMPTY;

export async function attachTranscriptListeners(): Promise<UnlistenFn> {
  return listen<TranscriptDelta>("transcript://delta", (e) => {
    useTranscriptStore.getState().apply(e.payload);
  });
}
