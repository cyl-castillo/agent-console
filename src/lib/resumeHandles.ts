// Adopting a terminal's resume handle from the CLI when no hook ever fired.
//
// A session becomes resumable (`claude --resume <id>`) the moment we learn its
// agent-side session id, and until now that only happened through the
// UserPromptSubmit hook. If the hook doesn't run — not installed yet, a
// directory the CLI won't trust, an agent the user started by hand — the
// terminal stays unresumable and a restart quietly opens a NEW conversation.
//
// `claude agents --json` lists live sessions with their pid, and the backend
// matches those pids against each PTY's own shell (see agent_sessions.rs), so a
// match is proof rather than a guess. This module is the frontend half: run the
// pass only when it could change something, and write the id where the hook
// would have written it.

import { profileFor } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import { useTerminalsStore } from "../stores/terminalsStore";

/// How often the pass runs while the app is open. The backend does nothing
/// when no terminal is missing an id, so a settled app pays nothing; the
/// interval only bounds how long a fresh unhooked session stays unresumable.
export const RESUME_HANDLE_POLL_MS = 60_000;

/// Terminals that could learn something from this pass: live, Claude-backed,
/// and with no id yet. Codex sessions are excluded on purpose — the ids come
/// from `claude agents`, and typing one into `codex resume <id>` would be a
/// confident lie.
function candidates(): string[] {
  return useTerminalsStore
    .getState()
    .sessions.filter((s) => s.status === "live")
    .filter((s) => !s.agentSessionId)
    .filter((s) => profileFor(s.agent).kind === "claude")
    .map((s) => s.id);
}

/// One reconciliation pass. Returns how many terminals adopted an id (0 when
/// there was nothing to do, which is the steady state). Never throws: this
/// runs on a timer, and a CLI that isn't there is a normal state, not an error
/// worth a toast.
export async function adoptLiveResumeHandles(): Promise<number> {
  const waiting = new Set(candidates());
  // Nothing to learn ⇒ don't even ask the backend, which would otherwise spawn
  // a `claude agents --json` every minute for the life of the app.
  if (waiting.size === 0) return 0;
  let bindings;
  try {
    bindings = await ipc.termAgentSessions();
  } catch {
    return 0;
  }
  let adopted = 0;
  for (const b of bindings) {
    // Re-check against `waiting`: the backend answers for every terminal it can
    // match, including ones the hook bound while this call was in flight.
    if (!b.sessionId || !waiting.has(b.termKey)) continue;
    useTerminalsStore.getState().setAgentSessionId(b.termKey, b.sessionId);
    adopted += 1;
  }
  return adopted;
}
