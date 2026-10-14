import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import { useProofStore } from "./proofStore";
import { useTerminalsStore } from "./terminalsStore";
import { useToastStore } from "./toastStore";
import type { TermInputDetail } from "../components/Terminal";

/// D13: before a session closes, the agent is consulted — asked for a
/// handoff note in its own words. The note is saved as a memory (advisory
/// context for the next session; it has no power to merge or run anything).
export const HANDOFF_PROMPT =
  "Before this session closes, write a handoff note for whoever continues: what was done, what is pending, what to watch out for, and anything you would want the next session to know. Under 200 words.";

interface Pending {
  sessionId: string;
  name: string;
  requestedAt: number;
}

interface HandoffState {
  pending: Pending[];
  /// Send the prompt into the live session and wait for its next turn_end.
  request: (sessionId: string) => boolean;
  /// A turn ended for a session with a pending request: save its summary.
  settle: (sessionId: string, ts: number, summary: string) => Promise<void>;
}

export const useHandoffStore = create<HandoffState>((set, get) => ({
  pending: [],
  request: (sessionId) => {
    const s = useTerminalsStore.getState().sessions.find((x) => x.id === sessionId);
    if (!s || s.status !== "live") return false;
    if (typeof window === "undefined") return false;
    const detail: TermInputDetail = { sessionId, data: `${HANDOFF_PROMPT}\r` };
    window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
    set({
      pending: [
        ...get().pending.filter((p) => p.sessionId !== sessionId),
        { sessionId, name: s.name, requestedAt: Date.now() },
      ],
    });
    useToastStore
      .getState()
      .show(
        `Asked ${s.name} for a handoff note — it is saved as a memory when the turn ends`,
        "info",
      );
    return true;
  },
  settle: async (sessionId, ts, summary) => {
    const p = get().pending.find((x) => x.sessionId === sessionId);
    if (!p || ts < p.requestedAt || !summary.trim()) return;
    set({ pending: get().pending.filter((x) => x.sessionId !== sessionId) });
    const day = new Date(ts).toISOString().slice(0, 10);
    const name = `handoff-${p.name}-${day}`;
    const content = `# Handoff from "${p.name}" (${day})\n\n${summary.trim()}\n`;
    try {
      await ipc.learningSaveMemory(name, content);
      useToastStore.getState().show(`Handoff note saved as memory ${name}`, "success");
    } catch (e) {
      useToastStore
        .getState()
        .show(`Couldn't save the handoff note: ${String(e).slice(0, 120)}`, "error");
    }
  },
}));

/// Watch the ledger: the first turn_end after a request, for that session,
/// carries the note.
export function attachHandoffWatcher(): () => void {
  let seen = 0;
  return useProofStore.subscribe((state) => {
    const events = state.events;
    for (const e of events) {
      if (e.seq <= seen) continue;
      if (e.kind === "turn_end" && e.termId) {
        const p = e.payload as { summary?: unknown };
        if (typeof p.summary === "string")
          void useHandoffStore.getState().settle(e.termId, e.ts, p.summary);
      }
    }
    seen = events.length ? events[events.length - 1].seq : seen;
  });
}
