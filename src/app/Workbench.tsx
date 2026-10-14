import { AdvisorPanel } from "../components/AdvisorPanel";
import { AgendaPanel } from "../components/AgendaPanel";
import { ContextPanel } from "../components/ContextPanel";
import { ExportImportPanel } from "../components/ExportImportPanel";
import { FeedbackPanel } from "../components/FeedbackPanel";
import { FileInspector } from "../components/FileInspector";
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
import { WorkbenchSubTabs, WorkbenchTabs } from "../components/WorkbenchTabs";
import { useUIStore } from "../stores/uiStore";
import { useWorkbenchStore } from "../stores/workbenchStore";

/// The right column. On the Changes tab it becomes the file inspector;
/// otherwise the workbench tabs and the active panel.
export function Workbench() {
  const centerTab = useUIStore((s) => s.tab);
  const tab = useWorkbenchStore((s) => s.tab);
  const setTab = useWorkbenchStore((s) => s.setTab);

  return (
    <aside className="panel right">
      {centerTab === "changes" ? (
        <FileInspector />
      ) : (
        <div className="workbench-layout">
          <WorkbenchTabs active={tab} onChange={setTab} />
          <div className="workbench-content">
            <WorkbenchSubTabs active={tab} onChange={setTab} />
            {tab === "skills" && <SkillsPanel />}
            {tab === "permissions" && <PermissionsPanel />}
            {tab === "advisor" && <AdvisorPanel />}
            {tab === "learning" && <LearningPanel />}
            {tab === "roundtable" && <RoundtablePanel />}
            {tab === "jobs" && <JobsPanel />}
            {tab === "schedule" && <SchedulerPanel />}
            {tab === "vault" && <VaultPanel />}
            {tab === "context" && <ContextPanel />}
            {tab === "jira" && <JiraPanel />}
            {tab === "teams" && <TeamsPanel />}
            {tab === "agenda" && <AgendaPanel />}
            {tab === "notes" && <NotesPanel />}
            {tab === "proof" && <ProofPanel />}
            {tab === "turns" && <TurnsPanel />}
            {tab === "plugins" && <PluginsPanel />}
            {tab === "mcp" && <McpPanel />}
            {tab === "transfer" && <ExportImportPanel />}
            {tab === "feedback" && <FeedbackPanel />}
          </div>
        </div>
      )}
    </aside>
  );
}
