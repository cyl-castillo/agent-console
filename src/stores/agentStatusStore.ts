import { create } from "zustand";

/// Glanceable agent activity for the status-bar pill.
///
/// Honesty note: the coding CLIs we drive (claude/codex) run inside the PTY and
/// give us a turn-START signal (the UserPromptSubmit hook) and approval requests,
/// but NO turn-END / "went idle" signal. So "working" can only be a best-effort
/// *recent-activity* indicator that decays: we bump it on agent-caused events
/// (prompt submitted, approval requested, approval answered) and let it fall back
/// to idle after a quiet window. It under-reports during long auto-approved
/// stretches (no events reach us) — that's the safe direction: it never falsely
/// claims the agent is still working. "Blocked" (an approval is pending) is the
/// one fully-reliable state and is derived separately from the approval queue.

export const WORKING_WINDOW_MS = 8000;

interface AgentStatusState {
  /// Epoch ms until which the agent counts as "recently active".
  workingUntil: number;
  markActive: () => void;
}

export const useAgentStatusStore = create<AgentStatusState>((set) => ({
  workingUntil: 0,
  markActive: () => set({ workingUntil: Date.now() + WORKING_WINDOW_MS }),
}));
