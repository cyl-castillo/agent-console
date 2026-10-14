import {
  Inbox,
  TerminalSquare,
  Briefcase,
  ShieldCheck,
  Settings2,
  type LucideIcon,
} from "lucide-react";
import type { WorkbenchTab } from "../../lib/workbenchTabs";

export type Destination = "inbox" | "sessions" | "work" | "proof" | "settings";

export interface DestinationDef {
  key: Destination;
  label: string;
  icon: LucideIcon;
  /// Sub-sections shown in the list pane, each a group of workbench tabs.
  sections: { label: string; tabs: WorkbenchTab[] }[];
}

/// The five destinations of the v2 shell (UI-2026-10.md §Arquitectura). The
/// 19 workbench tab ids stay the routing currency (palette, persistence,
/// deep links); a destination is where a tab is shown and which list the
/// left pane offers.
export const DESTINATIONS: DestinationDef[] = [
  { key: "inbox", label: "Inbox", icon: Inbox, sections: [] },
  { key: "sessions", label: "Sessions", icon: TerminalSquare, sections: [] },
  {
    key: "work",
    label: "Work",
    icon: Briefcase,
    sections: [
      { label: "Agents", tabs: ["jobs", "roundtable", "schedule"] },
      { label: "Queue", tabs: ["jira", "agenda", "teams"] },
      { label: "Notes", tabs: ["notes"] },
    ],
  },
  {
    key: "proof",
    label: "Proof",
    icon: ShieldCheck,
    sections: [{ label: "Witnessed", tabs: ["turns", "proof"] }],
  },
  {
    key: "settings",
    label: "Settings",
    icon: Settings2,
    sections: [
      { label: "Trust", tabs: ["permissions", "vault"] },
      { label: "Coach", tabs: ["skills", "advisor", "learning", "context"] },
      { label: "Add-ons", tabs: ["plugins", "mcp"] },
      { label: "Maintenance", tabs: ["transfer", "feedback"] },
    ],
  },
];

export const TAB_LABELS: Record<WorkbenchTab, string> = {
  jobs: "Jobs",
  roundtable: "Room",
  schedule: "Schedule",
  jira: "Jira queue",
  agenda: "Agenda",
  teams: "Teams",
  notes: "Notes",
  turns: "Turns",
  proof: "Ledger",
  permissions: "Permissions",
  vault: "Vault",
  skills: "Skills",
  advisor: "Advisor",
  learning: "Learning",
  context: "Context",
  plugins: "Plugins",
  mcp: "MCP servers",
  transfer: "Export / import",
  feedback: "Feedback",
};

export function destinationForTab(tab: WorkbenchTab): Destination {
  for (const d of DESTINATIONS) {
    if (d.sections.some((s) => s.tabs.includes(tab))) return d.key;
  }
  return "sessions";
}

export function firstTabOf(dest: Destination): WorkbenchTab | null {
  const d = DESTINATIONS.find((x) => x.key === dest);
  return d?.sections[0]?.tabs[0] ?? null;
}

export const destinationDef = (key: Destination) => DESTINATIONS.find((d) => d.key === key)!;
