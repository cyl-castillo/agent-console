/// THE list of workbench tabs — the single source of truth. The palette
/// navigation whitelist and the localStorage restore guard both derive from
/// it: three hand-copied lists drifted once already (the palette couldn't
/// open the Room tab because "roundtable" was missing from one of them).
/// Lives outside the component file so fast refresh keeps working.
export const WORKBENCH_TABS = [
  "skills", "permissions", "advisor", "learning", "roundtable", "schedule",
  "vault", "context", "plugins", "mcp", "transfer", "feedback", "jira",
  "agenda", "notes", "proof",
] as const;

export type WorkbenchTab = (typeof WORKBENCH_TABS)[number];

export function isWorkbenchTab(v: unknown): v is WorkbenchTab {
  return typeof v === "string" && (WORKBENCH_TABS as readonly string[]).includes(v);
}

/// Strip consolidation: the strip shows one button per GROUP; merged groups
/// open with an inner sub-switcher. Tab ids above remain the routing currency
/// (palette events, persisted active tab, onboarding deep links) — a group is
/// only how the strip presents them. "transfer" and "feedback" belong to no
/// group on purpose: they're occasional actions, reachable from the command
/// palette, not workspaces that earn a permanent button.
export const WORKBENCH_GROUPS = [
  { key: "tasks", tabs: ["jira", "agenda"] },
  { key: "notes", tabs: ["notes"] },
  { key: "proof", tabs: ["proof"] },
  { key: "context", tabs: ["context"] },
  { key: "coach", tabs: ["skills", "advisor", "learning"] },
  { key: "room", tabs: ["roundtable"] },
  { key: "schedule", tabs: ["schedule"] },
  { key: "trust", tabs: ["permissions", "vault"] },
  { key: "addons", tabs: ["plugins", "mcp"] },
] as const;

export type WorkbenchGroupKey = (typeof WORKBENCH_GROUPS)[number]["key"];

export function groupForTab(tab: WorkbenchTab) {
  return WORKBENCH_GROUPS.find((g) => (g.tabs as readonly string[]).includes(tab));
}
