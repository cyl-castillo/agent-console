import { create } from "zustand";

/// Glanceable agent activity for the status-bar pill.
///
/// Turn START comes from the UserPromptSubmit hook; turn END now comes from the
/// Stop hook (both Claude and Codex fire it when a turn completes), which flips
/// the pill to idle precisely via markIdle(). The decay window remains as the
/// FALLBACK for sessions where the Stop hook isn't installed/trusted yet:
/// "working" bumps on agent-caused events (prompt, approval traffic) and falls
/// back to idle after a quiet window — under-reporting rather than falsely
/// claiming activity. "Blocked" (an approval is pending) is derived separately
/// from the approval queue and is always reliable.

export const WORKING_WINDOW_MS = 8000;

/// Notification types (Claude's Notification hook) that mean "the agent is
/// blocked on the human at its own prompt". `idle_prompt` is NOT one: it means
/// the turn ended a minute ago and nobody typed — that's idle, not blocked.
export const WAITING_NOTIFICATIONS = new Set([
  "permission_prompt",
  "agent_needs_input",
  "elicitation_dialog",
  "elicitation_url_dialog",
]);

export interface WaitingOnHuman {
  /// The notification type that raised it.
  kind: string;
  since: number;
  /// Terminal the notification came from, when the hook carried it.
  termId?: string;
}

interface AgentStatusState {
  /// Epoch ms until which the agent counts as "recently active".
  workingUntil: number;
  /// When the current stretch of work started (first markActive after idle);
  /// drives the "working… 2m 34s" elapsed readout. 0 = not working.
  workingSince: number;
  /// The CLI said it is waiting on the human at its OWN prompt (Notification
  /// hook, T2) — the approvals bridge didn't answer, or isn't on. Cleared by
  /// any sign the agent moved on (prompt, tool result, approval decided).
  waiting: WaitingOnHuman | null;
  markActive: () => void;
  /// A turn finished (Stop hook) — drop to idle now, don't wait out the decay.
  markIdle: () => void;
  /// A Notification hook event. Sets `waiting` for the blocking kinds, treats
  /// `idle_prompt` as a turn end, ignores the rest.
  noteNotification: (kind: string | undefined, termId?: string) => void;
}

export const useAgentStatusStore = create<AgentStatusState>((set, get) => ({
  workingUntil: 0,
  workingSince: 0,
  waiting: null,
  markActive: () => {
    const now = Date.now();
    const wasWorking = now < get().workingUntil;
    set({
      workingUntil: now + WORKING_WINDOW_MS,
      workingSince: wasWorking && get().workingSince > 0 ? get().workingSince : now,
      // Activity means the agent got past whatever it was waiting for.
      waiting: null,
    });
  },
  markIdle: () => set({ workingUntil: 0, workingSince: 0, waiting: null }),
  noteNotification: (kind, termId) => {
    if (!kind) return;
    if (WAITING_NOTIFICATIONS.has(kind)) {
      set({ waiting: { kind, since: Date.now(), termId } });
    } else if (kind === "idle_prompt") {
      // A minute of silence after the turn: whatever Stop said, it's idle.
      get().markIdle();
    }
  },
}));
