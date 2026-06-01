import { create } from "zustand";
import type { AgentKind } from "../agents/profiles";
import { isValidModel } from "../agents/profiles";

/// Per-project remembered choices for new sessions: which agent to launch and,
/// for that agent, which model/tuning preset. A "model" here is just the string
/// the agent profile feeds into its launch command — we track *intent* (what the
/// user last asked for), never the agent's actually-loaded model.
///
/// Model defaults are keyed by (project, agent) so switching agent doesn't carry
/// the wrong value across (Claude's "opus" must not leak into Codex).

export type { AgentKind } from "../agents/profiles";
export { isValidModel, modelLabel, AGENT_PROFILES, profileFor, DEFAULT_AGENT } from "../agents/profiles";

const MODEL_KEY_PREFIX = "agent-console:model:";
const AGENT_KEY_PREFIX = "agent-console:agent:";

function readAgent(root: string): AgentKind | undefined {
  try {
    const v = localStorage.getItem(AGENT_KEY_PREFIX + root);
    return v === "claude" || v === "codex" ? v : undefined;
  } catch {
    return undefined;
  }
}

function readModel(root: string, agent: AgentKind): string | undefined {
  try {
    const v = localStorage.getItem(`${MODEL_KEY_PREFIX}${agent}:${root}`);
    return isValidModel(v) ? v : undefined;
  } catch {
    return undefined;
  }
}

interface ModelState {
  /// (root|agent) -> model cache mirroring localStorage; "" sentinel = explicit default.
  modelDefaults: Record<string, string | undefined>;
  /// root -> agent cache.
  agentDefaults: Record<string, AgentKind | undefined>;

  /// Last picked model for (project, agent), or undefined if never picked.
  defaultFor: (root: string, agent: AgentKind) => string | undefined;
  /// Remember the user's model choice as the (project, agent) default.
  setDefaultFor: (root: string, agent: AgentKind, model: string | undefined) => void;
  /// Last picked agent for this project, or undefined if never picked.
  defaultAgentFor: (root: string) => AgentKind | undefined;
  /// Remember the user's agent choice as the project default.
  setDefaultAgentFor: (root: string, agent: AgentKind) => void;
}

export const useModelStore = create<ModelState>((set, get) => ({
  modelDefaults: {},
  agentDefaults: {},

  // Pure reads (safe during render): cache first, then localStorage.
  defaultFor: (root, agent) => {
    const k = `${agent}|${root}`;
    const cached = get().modelDefaults[k];
    return cached !== undefined ? cached : readModel(root, agent);
  },

  setDefaultFor: (root, agent, model) => {
    const storageKey = `${MODEL_KEY_PREFIX}${agent}:${root}`;
    try {
      if (isValidModel(model)) localStorage.setItem(storageKey, model);
      else localStorage.removeItem(storageKey);
    } catch {
      /* ignore */
    }
    const k = `${agent}|${root}`;
    set((s) => ({ modelDefaults: { ...s.modelDefaults, [k]: isValidModel(model) ? model : undefined } }));
  },

  defaultAgentFor: (root) => {
    const cached = get().agentDefaults[root];
    return cached !== undefined ? cached : readAgent(root);
  },

  setDefaultAgentFor: (root, agent) => {
    try {
      localStorage.setItem(AGENT_KEY_PREFIX + root, agent);
    } catch {
      /* ignore */
    }
    set((s) => ({ agentDefaults: { ...s.agentDefaults, [root]: agent } }));
  },
}));
