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

interface AgentStatusState {
  /// Epoch ms until which the agent counts as "recently active".
  workingUntil: number;
  /// When the current stretch of work started (first markActive after idle);
  /// drives the "working… 2m 34s" elapsed readout. 0 = not working.
  workingSince: number;
  markActive: () => void;
  /// A turn finished (Stop hook) — drop to idle now, don't wait out the decay.
  markIdle: () => void;
}

export const useAgentStatusStore = create<AgentStatusState>((set, get) => ({
  workingUntil: 0,
  workingSince: 0,
  markActive: () => {
    const now = Date.now();
    const wasWorking = now < get().workingUntil;
    set({
      workingUntil: now + WORKING_WINDOW_MS,
      workingSince: wasWorking && get().workingSince > 0 ? get().workingSince : now,
    });
  },
  markIdle: () => set({ workingUntil: 0, workingSince: 0 }),
}));
