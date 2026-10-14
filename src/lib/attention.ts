import type { AgentState } from "../ui/StatusGlyph";

/// Minimal shape of what the attention rules read, so the function stays
/// pure and testable without the stores.
export interface AttentionInputs {
  session: { id: string; status: "live" | "stopped"; archived?: boolean };
  /// Pending approvals' termIds (approvalStore.queue).
  approvalTermIds: string[];
  /// The CLI said it is waiting on the human at its own prompt.
  waitingTermId?: string | null;
  /// Global "working" window (agentStatusStore) and the terminal it is
  /// attributed to — the hooks only carry a termId some of the time, so the
  /// active session owns the window when nothing else does.
  workingUntil: number;
  workingTermId?: string | null;
  activeId: string | null;
  now: number;
  /// The ledger shows a turn that started and has not ended (prompt without
  /// turn_end, with recent events) — the agent is working whatever the
  /// status hooks say.
  openTurn?: boolean;
}

/// The attention state of one session, in priority order: it needs you
/// (approval or the agent's own prompt) → it is working → it stopped → idle
/// ("done" in the five-state vocabulary: nothing is happening and nothing is
/// asked).
export function sessionAttention(i: AttentionInputs): AgentState {
  const { session } = i;
  if (session.status === "stopped") return "stopped";
  if (i.approvalTermIds.includes(session.id)) return "needs-you";
  if (i.waitingTermId && i.waitingTermId === session.id) return "needs-you";
  if (i.openTurn) return "working";
  const working = i.now < i.workingUntil;
  if (working && (i.workingTermId ? i.workingTermId === session.id : i.activeId === session.id)) {
    return "working";
  }
  return "done";
}

export const ATTENTION_ORDER: AgentState[] = [
  "needs-you",
  "review",
  "working",
  "done",
  "failed",
  "stopped",
];
