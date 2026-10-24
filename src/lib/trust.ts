import type { TrustStatus } from "../types/domain";

/// F4: does this folder still need the human's word before an agent starts
/// in it? Undecided, or decided on a surface that has since changed.
export function trustNeedsDecision(status: TrustStatus | null | undefined): boolean {
  return (
    status !== null &&
    status !== undefined &&
    (status.state === "undecided" || status.state === "changed")
  );
}

/// Agents may auto-launch only on a trusted folder; "declined" leaves the
/// launch to the human's own hands.
export function trustAllowsLaunch(status: TrustStatus | null | undefined): boolean {
  return status?.state === "trusted";
}

/// What the card says the folder would run: the executing categories first.
export function surfaceSummary(status: TrustStatus): string {
  const c = status.surface.counts;
  const parts: string[] = [];
  const add = (n: number, one: string, many: string) => {
    if (n > 0) parts.push(`${n} ${n === 1 ? one : many}`);
  };
  add(c.hooks, "hook file", "hook files");
  add(c.setup, "setup command", "setup commands");
  add(c.plugins, "plugin", "plugins");
  add(c.mcp, "MCP config", "MCP configs");
  add(c.rules, "rules file", "rules files");
  add(c.skills, "skill", "skills");
  add(c.instructions, "instructions file", "instructions files");
  return parts.length ? parts.join(" · ") : "nothing that shapes the agents";
}

/// Executing files make it a high-tier decision; instructions alone are medium.
export function trustTier(status: TrustStatus): "high" | "medium" {
  const c = status.surface.counts;
  return c.hooks + c.setup + c.plugins + c.mcp > 0 ? "high" : "medium";
}
