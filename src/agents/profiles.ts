/// Agent profiles — the single place that knows how each supported coding agent
/// is launched, which "models" it exposes, and what it can/can't do. The rest of
/// the app stays agent-neutral: it carries an `agent` kind per session and asks
/// the profile to build the launch command and supply the model presets.
///
/// To add a third agent later, add one entry to PROFILES — nothing else in the
/// UI hardcodes a specific CLI.

export type AgentKind = "claude" | "codex";

/// Default when a session predates the agent field (all old sessions were Claude).
export const DEFAULT_AGENT: AgentKind = "claude";

/// A model/tuning preset surfaced in the chooser and the status-bar pill. `value`
/// is what we store on the session and feed back into buildLaunch; it must be
/// shell-safe (validated by isValidModel before it ever reaches a PTY command).
export interface AgentModelPreset {
  value: string;
  /// Short label for the value itself (shown in the pill).
  label: string;
  /// What you'd use it for — the primary line in the picker.
  intent: string;
  /// Leading glyph for quick visual scanning.
  icon: string;
}

/// Everything Terminal needs to type the (re)launch command into the PTY.
export interface AgentLaunch {
  cmd: string;
  label: string;
  note: string;
}

/// Minimal session shape buildLaunch needs — kept structural to avoid a cyclic
/// import with terminalsStore (which imports AgentKind from here).
export interface LaunchContext {
  /// Captured agent session id (Claude only, via the UserPromptSubmit hook).
  agentSessionId?: string;
  /// Chosen model/tuning value, or undefined for the account/config default.
  model?: string;
  /// Whether this terminal has prior scrollback (affects the "fresh"/"resuming" note).
  hasScrollback: boolean;
}

export interface AgentProfile {
  kind: AgentKind;
  /// Human-facing name ("Claude", "Codex").
  label: string;
  /// Glyph for the agent chooser.
  icon: string;
  /// Bare binary name — resolved from the user's PATH by the login shell that
  /// runs the PTY (we type this as text, we do not spawn it ourselves).
  binName: string;
  /// Model/tuning presets for the chooser and the pill.
  models: AgentModelPreset[];
  /// Whether picking a model on a *live* session can be pushed into the running
  /// agent (Claude accepts `/model`; Codex does not, so we only re-launch later).
  supportsLiveModelSwitch: boolean;
  /// The input to send to a live agent to switch model (only if supported).
  liveModelSwitchInput?: (model: string) => string;
  /// Build the command typed into the PTY when this terminal spawns/resumes.
  buildLaunch: (ctx: LaunchContext) => AgentLaunch;
}

const CLAUDE_MODELS: AgentModelPreset[] = [
  { value: "opus", label: "Opus", intent: "Plan / architecture", icon: "🧠" },
  { value: "sonnet", label: "Sonnet", intent: "Implement", icon: "🔨" },
  { value: "haiku", label: "Haiku", intent: "Quick edits / format", icon: "⚡" },
];

/// Codex tunes a single model via reasoning effort rather than swapping models,
/// so the "model" we store is the effort level and we encode it as a config
/// override. This avoids guessing model ids while still giving the same
/// plan/implement/quick spectrum as Claude.
const CODEX_MODELS: AgentModelPreset[] = [
  { value: "high", label: "High effort", intent: "Plan / architecture", icon: "🧠" },
  { value: "medium", label: "Medium effort", intent: "Implement", icon: "🔨" },
  { value: "low", label: "Low effort", intent: "Quick edits / format", icon: "⚡" },
];

const CLAUDE: AgentProfile = {
  kind: "claude",
  label: "Claude",
  icon: "✶",
  binName: "claude",
  models: CLAUDE_MODELS,
  supportsLiveModelSwitch: true,
  liveModelSwitchInput: (m) => `/model ${m}\r`,
  buildLaunch: (ctx) => {
    // Only do precise --resume when we captured a session id (from the hook).
    // Never `claude --continue`: it picks the LAST claude session globally, so
    // multiple stopped terminals would all converge on the same conversation.
    let cmd: string;
    let label: string;
    let note: string;
    if (ctx.agentSessionId) {
      cmd = `claude --resume ${ctx.agentSessionId}`;
      label = `claude (${ctx.agentSessionId.slice(0, 8)}…)`;
      note = "auto-resuming";
    } else {
      cmd = "claude";
      label = "claude";
      note = ctx.hasScrollback ? "starting fresh (no session id)" : "starting";
    }
    if (isValidModel(ctx.model)) {
      cmd += ` --model ${ctx.model}`;
      label += ` · ${ctx.model}`;
    }
    return { cmd, label, note };
  },
};

const CODEX: AgentProfile = {
  kind: "codex",
  label: "Codex",
  icon: "◆",
  binName: "codex",
  models: CODEX_MODELS,
  // Codex has no `/model` slash command we can push reliably; the choice only
  // takes effect on (re)launch.
  supportsLiveModelSwitch: false,
  buildLaunch: (ctx) => {
    // We can't auto-resume by id: capturing it needs a UserPromptSubmit-style
    // hook, which Codex doesn't expose to us yet. Start fresh — the user can run
    // `codex resume` inside the TUI to pick a prior session. (Avoid
    // `codex resume --last`: like `claude --continue` it's global and would make
    // several terminals converge on the same conversation.)
    let cmd = "codex";
    let label = "codex";
    const note = ctx.hasScrollback ? "starting fresh" : "starting";
    // Encode the chosen effort as a config override rather than a model id.
    if (isValidModel(ctx.model)) {
      cmd += ` -c model_reasoning_effort=${ctx.model}`;
      label += ` · ${ctx.model}`;
    }
    return { cmd, label, note };
  },
};

const PROFILES: Record<AgentKind, AgentProfile> = {
  claude: CLAUDE,
  codex: CODEX,
};

/// All profiles in display order (used by the agent chooser).
export const AGENT_PROFILES: AgentProfile[] = [CLAUDE, CODEX];

/// Resolve a profile, falling back to the default agent for undefined/unknown.
export function profileFor(agent: AgentKind | undefined): AgentProfile {
  return PROFILES[agent ?? DEFAULT_AGENT] ?? PROFILES[DEFAULT_AGENT];
}

/// Narrow a persisted/free-form string to a known AgentKind, or undefined.
export function asAgentKind(s: string | undefined | null): AgentKind | undefined {
  return s === "claude" || s === "codex" ? s : undefined;
}

/// Model/tuning values are interpolated into a shell command written to the PTY,
/// so they must be shell-safe. Presets and full ids are all `[A-Za-z0-9._-]+`;
/// anything else is rejected (treated as "no model").
const MODEL_RE = /^[A-Za-z0-9._-]+$/;

export function isValidModel(model: string | undefined | null): model is string {
  return typeof model === "string" && model.length > 0 && model.length <= 64 && MODEL_RE.test(model);
}

/// Human-facing label for a model value within a given agent. Known presets get
/// their label; custom values are shown verbatim.
export function modelLabel(model: string | undefined | null, agent?: AgentKind): string {
  if (!model) return "default";
  const preset = profileFor(agent).models.find((p) => p.value === model);
  return preset ? preset.label : model;
}
