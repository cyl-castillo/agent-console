import { useEffect } from "react";

import { profileFor, type AgentKind, type AgentModelPreset } from "./profiles";
import { useOpencodeModelsStore } from "../stores/opencodeModelsStore";
import { useSessionStore } from "../stores/sessionStore";
import type { OpencodeModel } from "../types/domain";

/// The model presets a picker offers for `agent`: the profile's static list
/// for Claude and Codex, OpenCode's live catalog (already loaded) otherwise.
/// Pure read — safe during render and from stores.
export function presetsFor(agent: AgentKind | undefined): AgentModelPreset[] {
  const profile = profileFor(agent);
  if (profile.kind !== "opencode") return profile.models;
  return useOpencodeModelsStore.getState().models.map(toPreset);
}

function toPreset(m: OpencodeModel): AgentModelPreset {
  return { value: m.id, label: m.name, intent: m.provider, icon: "⬡" };
}

/// Presets for `agent`, loading OpenCode's catalog for the open project the
/// first time it is asked for. Components use this; stores use `presetsFor`.
export function useModelPresets(agent: AgentKind | undefined): AgentModelPreset[] {
  const root = useSessionStore((s) => s.project?.root);
  const models = useOpencodeModelsStore((s) => s.models);
  const load = useOpencodeModelsStore((s) => s.load);
  const profile = profileFor(agent);
  useEffect(() => {
    if (profile.kind === "opencode" && root) void load(root);
  }, [profile.kind, root, load]);
  if (profile.kind !== "opencode") return profile.models;
  return models.map(toPreset);
}
