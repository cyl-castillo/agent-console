/// Workbench modularity: every strip GROUP is a module the user can switch
/// off. "Off" only hides — the strip button, its palette actions, the restored
/// tab — it never deletes data or stops the backend. The registry derives from
/// WORKBENCH_GROUPS so a new group cannot ship without deciding its module
/// story: the Record below fails to compile until the new key gets an entry.

import {
  WORKBENCH_GROUPS,
  groupForTab,
  type WorkbenchGroupKey,
  type WorkbenchTab,
} from "./workbenchTabs";

export interface ModuleInfo {
  key: WorkbenchGroupKey;
  label: string;
  description: string;
  /// Locked modules cannot be switched off. Trust (permissions + vault) is
  /// the only one: what the agent may touch is safety, not a feature.
  locked: boolean;
}

const META: Record<WorkbenchGroupKey, { label: string; description: string; locked?: boolean }> = {
  tasks: { label: "Tasks", description: "Jira queue and the unified agenda" },
  teams: { label: "Teams", description: "Read the Microsoft Teams messages sent to you" },
  notes: { label: "Notes", description: "Per-project sticky-note scratchpad" },
  proof: { label: "Proof", description: "Verifiable proof packets for your PRs" },
  context: { label: "Context", description: "CLAUDE.md and project memories" },
  coach: { label: "Coach", description: "Skills, advisor suggestions and learning" },
  room: { label: "Room", description: "Multi-agent conversation room" },
  schedule: { label: "Schedule", description: "Agent jobs on a clock (suggest-only)" },
  trust: {
    label: "Trust",
    description: "Permissions and vault — what the agent can touch",
    locked: true,
  },
  addons: { label: "Add-ons", description: "Plugins and MCP servers" },
};

export const MODULES: ModuleInfo[] = WORKBENCH_GROUPS.map((g) => ({
  key: g.key,
  label: META[g.key].label,
  description: META[g.key].description,
  locked: !!META[g.key].locked,
}));

/// The module a tab belongs to; null for groupless tabs (transfer, feedback),
/// which are palette-only and always available.
export function moduleForTab(tab: WorkbenchTab): WorkbenchGroupKey | null {
  return groupForTab(tab)?.key ?? null;
}
