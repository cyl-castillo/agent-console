import { create } from "zustand";

/// Model selection for agent sessions. Claude Code runs inside the integrated
/// terminal, so a "model" here is just the string we pass to `claude --model`
/// at launch (or send via `/model <m>` mid-session). We never read Claude's
/// actual loaded model back — the value tracks *intent*, i.e. what the user
/// last asked for.
///
/// A model is undefined until the user explicitly picks one, in which case the
/// session launches exactly as before (`claude` with no `--model`, using the
/// account default). Once picked, the choice is remembered per project.

/// Built-in aliases the CLI accepts (`claude --model opus|sonnet|haiku`).
export type ModelAlias = "opus" | "sonnet" | "haiku";

/// Task-intent presets: the user thinks in terms of *what they're about to do*,
/// each mapped to the model that fits. The raw model name is shown alongside so
/// it doubles as a model picker.
export interface ModelPreset {
  /// Stable key, also used as the value stored on the session.
  model: ModelAlias;
  /// Short label for the model itself (shown in the pill).
  label: string;
  /// What you'd use it for — the primary line in the picker.
  intent: string;
  /// Leading glyph for quick visual scanning.
  icon: string;
}

export const MODEL_PRESETS: ModelPreset[] = [
  { model: "opus", label: "Opus", intent: "Plan / architecture", icon: "🧠" },
  { model: "sonnet", label: "Sonnet", intent: "Implement", icon: "🔨" },
  { model: "haiku", label: "Haiku", intent: "Quick edits / format", icon: "⚡" },
];

/// `claude --model <m>` is interpolated into a shell command written to the
/// PTY, so the value must be shell-safe. Aliases and full model ids are all
/// `[A-Za-z0-9._-]+`; anything else is rejected (treated as "no model").
const MODEL_RE = /^[A-Za-z0-9._-]+$/;

export function isValidModel(model: string | undefined | null): model is string {
  return typeof model === "string" && model.length > 0 && model.length <= 64 && MODEL_RE.test(model);
}

/// Human-facing label for a model string. Known aliases get their preset label;
/// custom full ids are shown verbatim.
export function modelLabel(model: string | undefined | null): string {
  if (!model) return "default";
  const preset = MODEL_PRESETS.find((p) => p.model === model);
  return preset ? preset.label : model;
}

const KEY_PREFIX = "agent-console:model:";

function readDefault(root: string): string | undefined {
  try {
    const v = localStorage.getItem(KEY_PREFIX + root);
    return isValidModel(v) ? v : undefined;
  } catch {
    return undefined;
  }
}

interface ModelState {
  /// Per-project remembered default (root -> model). undefined = account default.
  defaults: Record<string, string | undefined>;
  /// Last picked model for this project, or undefined if never picked.
  defaultFor: (root: string) => string | undefined;
  /// Remember the user's choice as the project default.
  setDefaultFor: (root: string, model: string | undefined) => void;
}

export const useModelStore = create<ModelState>((set, get) => ({
  defaults: {},

  // Pure read (safe to call during render): cache first, then localStorage.
  // The cache is only written by setDefaultFor, never here.
  defaultFor: (root) => {
    const cached = get().defaults[root];
    return cached !== undefined ? cached : readDefault(root);
  },

  setDefaultFor: (root, model) => {
    try {
      if (isValidModel(model)) localStorage.setItem(KEY_PREFIX + root, model);
      else localStorage.removeItem(KEY_PREFIX + root);
    } catch {
      /* ignore */
    }
    set((s) => ({ defaults: { ...s.defaults, [root]: isValidModel(model) ? model : undefined } }));
  },
}));
