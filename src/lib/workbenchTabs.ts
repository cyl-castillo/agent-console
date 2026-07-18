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
