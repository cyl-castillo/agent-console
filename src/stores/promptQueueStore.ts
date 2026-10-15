import { create } from "zustand";

/// Prompts queued for later (Ctrl+Enter): one list per session, sent one at
/// a time by the conversation view when the session has no live turn. Not
/// persisted — a queue is an intention for this sitting, and a restart
/// should not fire prompts nobody is watching.
export interface QueuedPrompt {
  id: string;
  text: string;
  ts: number;
}

interface PromptQueueState {
  bySession: Record<string, QueuedPrompt[]>;
  enqueue: (sessionId: string, text: string) => QueuedPrompt;
  remove: (sessionId: string, id: string) => void;
  /// Take the oldest queued prompt of a session, if any.
  shift: (sessionId: string) => QueuedPrompt | undefined;
  clear: (sessionId: string) => void;
}

let seq = 0;

export const usePromptQueueStore = create<PromptQueueState>((set, get) => ({
  bySession: {},
  enqueue: (sessionId, text) => {
    const item: QueuedPrompt = { id: `q-${Date.now()}-${++seq}`, text, ts: Date.now() };
    set((s) => ({
      bySession: { ...s.bySession, [sessionId]: [...(s.bySession[sessionId] ?? []), item] },
    }));
    return item;
  },
  remove: (sessionId, id) =>
    set((s) => ({
      bySession: {
        ...s.bySession,
        [sessionId]: (s.bySession[sessionId] ?? []).filter((q) => q.id !== id),
      },
    })),
  shift: (sessionId) => {
    const list = get().bySession[sessionId] ?? [];
    const [first, ...rest] = list;
    if (!first) return undefined;
    set((s) => ({ bySession: { ...s.bySession, [sessionId]: rest } }));
    return first;
  },
  clear: (sessionId) => set((s) => ({ bySession: { ...s.bySession, [sessionId]: [] } })),
}));

const EMPTY: QueuedPrompt[] = [];

/// The queue of one session. A stable empty array when none: this is used
/// as a store selector, and a fresh `[]` per call would re-render forever.
export function queueFor(
  bySession: Record<string, QueuedPrompt[]>,
  sessionId: string | undefined,
): QueuedPrompt[] {
  return sessionId ? (bySession[sessionId] ?? EMPTY) : EMPTY;
}
