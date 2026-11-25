import { DEFAULT_AGENT, type AgentKind } from "../agents/profiles";
import { useModelStore } from "../stores/modelStore";
import { firstEngineFound, usePreflightStore } from "../stores/preflightStore";
import type { Preflight } from "../types/domain";

/// The engine of a session nobody picked one for: what was last used in this
/// folder, else the first agent CLI the setup check found, else Claude.
/// "Setup ready" on a machine with only Codex or OpenCode must not be
/// followed by a session that types `claude`.
export function pickInitialAgent(
  remembered: AgentKind | undefined,
  preflight: Preflight | null,
): AgentKind {
  return remembered ?? firstEngineFound(preflight) ?? DEFAULT_AGENT;
}

/// `pickInitialAgent` read from the stores, for callers outside React.
export function initialAgentFor(root: string): AgentKind {
  return pickInitialAgent(
    useModelStore.getState().defaultAgentFor(root),
    usePreflightStore.getState().result,
  );
}

/// The `agent` argument for a session that starts with `initialAgentFor`.
/// Claude stays unset, as it always was for a session opened without a
/// choice: an unset agent is Claude everywhere.
export function initialAgentArg(root: string): AgentKind | undefined {
  const agent = initialAgentFor(root);
  return agent === DEFAULT_AGENT ? undefined : agent;
}
