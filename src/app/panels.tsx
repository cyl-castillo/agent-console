import { AdvisorPanel } from "../components/AdvisorPanel";
import { AgendaPanel } from "../components/AgendaPanel";
import { ContextPanel } from "../components/ContextPanel";
import { ExportImportPanel } from "../components/ExportImportPanel";
import { FeedbackPanel } from "../components/FeedbackPanel";
import { JiraPanel } from "../components/JiraPanel";
import { JobsPanel } from "../components/JobsPanel";
import { LearningPanel } from "../components/LearningPanel";
import { McpPanel } from "../components/McpPanel";
import { NotesPanel } from "../components/NotesPanel";
import { PermissionsPanel } from "../components/PermissionsPanel";
import { PluginsPanel } from "../components/PluginsPanel";
import { ProofPanel } from "../components/ProofPanel";
import { RoundtablePanel } from "../components/RoundtablePanel";
import { SchedulerPanel } from "../components/SchedulerPanel";
import { SkillsPanel } from "../components/SkillsPanel";
import { TeamsPanel } from "../components/TeamsPanel";
import { TurnsPanel } from "../components/TurnsPanel";
import { VaultPanel } from "../components/VaultPanel";
import type { WorkbenchTab } from "../lib/workbenchTabs";

/// The one place that maps a workbench tab id to its panel component; both
/// shells (v1 workbench column, v2 destinations) render through it.
export function WorkbenchPanel({ tab }: { tab: WorkbenchTab }) {
  switch (tab) {
    case "skills":
      return <SkillsPanel />;
    case "permissions":
      return <PermissionsPanel />;
    case "advisor":
      return <AdvisorPanel />;
    case "learning":
      return <LearningPanel />;
    case "roundtable":
      return <RoundtablePanel />;
    case "jobs":
      return <JobsPanel />;
    case "schedule":
      return <SchedulerPanel />;
    case "vault":
      return <VaultPanel />;
    case "context":
      return <ContextPanel />;
    case "jira":
      return <JiraPanel />;
    case "teams":
      return <TeamsPanel />;
    case "agenda":
      return <AgendaPanel />;
    case "notes":
      return <NotesPanel />;
    case "proof":
      return <ProofPanel />;
    case "turns":
      return <TurnsPanel />;
    case "plugins":
      return <PluginsPanel />;
    case "mcp":
      return <McpPanel />;
    case "transfer":
      return <ExportImportPanel />;
    case "feedback":
      return <FeedbackPanel />;
  }
}
