import { useEffect, useState } from "react";

import { useSessionStore } from "./stores/sessionStore";
import { useChangesStore } from "./stores/changesStore";
import { usePreviewStore } from "./stores/previewStore";
import { useUIStore } from "./stores/uiStore";
import { attachSkillsListeners, useSkillsStore } from "./stores/skillsStore";
import { attachApprovalListener } from "./stores/approvalStore";
import { useUpdaterStore } from "./stores/updaterStore";
import { useTerminalsStore } from "./stores/terminalsStore";
import { ProjectPicker } from "./components/ProjectPicker";
import { LeftSidebar } from "./components/LeftSidebar";
import { Terminal } from "./components/Terminal";
import { ChangesView } from "./components/ChangesView";
import { Preview } from "./components/Preview";
import { SkillsPanel } from "./components/SkillsPanel";
import { PermissionsPanel } from "./components/PermissionsPanel";
import { WorkbenchTabs } from "./components/WorkbenchTabs";
import { ApprovalModal } from "./components/ApprovalModal";
import { FileInspector } from "./components/FileInspector";
import { AboutModal } from "./components/AboutModal";
import { UpdateBanner } from "./components/UpdateBanner";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { ipc } from "./ipc/tauri";
import type { WorkspaceContext } from "./types/domain";

export default function App() {
  const { project, closeProject } = useSessionStore();
  const tab = useUIStore((s) => s.tab);
  const setTab = useUIStore((s) => s.setTab);
  const changesCount = useChangesStore((s) => s.status?.changes.length ?? 0);
  const refreshChanges = useChangesStore((s) => s.refresh);
  const clearChanges = useChangesStore((s) => s.clear);
  const clearPreview = usePreviewStore((s) => s.clear);
  const refreshSkills = useSkillsStore((s) => s.refresh);
  const branch = useChangesStore((s) => s.status?.branch ?? null);
  const terminalSessions = useTerminalsStore((s) => s.sessions);
  const activeTerminalId = useTerminalsStore((s) => s.activeId);
  const hydrateTerminals = useTerminalsStore((s) => s.hydrate);
  const clearTerminals = useTerminalsStore((s) => s.clear);
  const persistTerminals = useTerminalsStore((s) => s.persist);
  const addTerminal = useTerminalsStore((s) => s.add);
  const [workspace, setWorkspace] = useState<WorkspaceContext | null>(null);
  const [showAbout, setShowAbout] = useState(false);
  const [workbenchTab, setWorkbenchTab] = useState<"skills" | "permissions">("skills");
  const [leftOpen, setLeftOpen] = useState(false);
  const checkForUpdates = useUpdaterStore((s) => s.check);

  useKeyboardShortcuts({ setTab });

  useEffect(() => {
    let offSkills: (() => void) | null = null;
    let offApproval: (() => void) | null = null;
    attachSkillsListeners().then((u) => { offSkills = u; });
    attachApprovalListener().then((u) => { offApproval = u; });
    return () => { offSkills?.(); offApproval?.(); };
  }, []);

  useEffect(() => {
    checkForUpdates({ silentIfNone: true });
  }, [checkForUpdates]);

  // Reload git/skills/workspace and hydrate sessions when project changes.
  useEffect(() => {
    if (!project) {
      clearChanges();
      clearPreview();
      clearTerminals();
      setWorkspace(null);
      return;
    }
    refreshChanges();
    refreshSkills();
    ipc.workspaceContext().then(setWorkspace).catch(() => setWorkspace(null));
    (async () => {
      await hydrateTerminals(project.root);
      // Auto-spawn one live session if nothing was restored from disk.
      if (useTerminalsStore.getState().sessions.length === 0) {
        addTerminal(project.root);
      }
    })();
  }, [project, refreshChanges, refreshSkills, clearChanges, clearPreview, hydrateTerminals, clearTerminals, addTerminal]);

  // Persist sessions on tab close / app unload.
  useEffect(() => {
    const onUnload = () => { persistTerminals(); };
    window.addEventListener("beforeunload", onUnload);
    return () => window.removeEventListener("beforeunload", onUnload);
  }, [persistTerminals]);

  // Periodic persist so a hard kill doesn't lose more than ~10s of scrollback.
  useEffect(() => {
    if (!project) return;
    const t = setInterval(() => { persistTerminals(); }, 10_000);
    return () => clearInterval(t);
  }, [project, persistTerminals]);

  const liveTerminals = terminalSessions.filter((s) => s.status === "live");

  if (!project) {
    return (
      <>
        <ProjectPicker />
        {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}
        <UpdateBanner />
      </>
    );
  }

  return (
    <>
      <div className={`app ${leftOpen ? "" : "left-collapsed"}`}>
        <div className="topbar">
          <button
            className={`sidebar-toggle ${leftOpen ? "open" : ""}`}
            onClick={() => setLeftOpen((v) => !v)}
            title={leftOpen ? "Hide workspace" : "Show workspace"}
          >
            <span className="sidebar-toggle-icon">{leftOpen ? "◧" : "◨"}</span>
            <span>Workspace</span>
            {changesCount > 0 && <span className="sidebar-toggle-badge">{changesCount}</span>}
          </button>
          <span className="title">{project.name}</span>
          <span className="meta">
            {project.language ?? "unknown"}
            {project.framework ? ` · ${project.framework}` : ""}
            {branch ? ` · ${branch}` : ""}
            {workspace?.fileCount ? ` · ${workspace.fileCount} files` : ""}
          </span>
          <span className="meta" style={{ opacity: 0.6 }}>{project.root}</span>
          <span className="spacer" />
          <button className="topbar-icon" onClick={() => setShowAbout(true)} title="About Agent Console">ⓘ</button>
          <button onClick={() => { persistTerminals(); closeProject(); }}>Close</button>
        </div>

        {leftOpen && <LeftSidebar />}

        <main className="panel center">
          <div className="tabs">
            <button
              className={`tab ${tab === "terminal" ? "active" : ""}`}
              onClick={() => setTab("terminal")}
              title="Ctrl+1"
            >
              Terminal
            </button>
            <button
              className={`tab ${tab === "changes" ? "active" : ""}`}
              onClick={() => setTab("changes")}
              title="Ctrl+2"
            >
              Changes
              {changesCount > 0 && <span className="tab-badge">{changesCount}</span>}
            </button>
            <button
              className={`tab ${tab === "preview" ? "active" : ""}`}
              onClick={() => setTab("preview")}
              title="Ctrl+3"
            >
              Preview
            </button>
          </div>
          <div className="tab-pane" style={{ display: tab === "terminal" ? "flex" : "none" }}>
            {liveTerminals.length === 0 ? (
              <div className="placeholder" style={{ padding: 16 }}>
                No active session. Click <strong>+ new</strong> in the Sessions panel.
              </div>
            ) : (
              <div className="terminals-stack">
                {liveTerminals.map((s) => (
                  <Terminal
                    key={s.id}
                    session={s}
                    visible={tab === "terminal" && s.id === activeTerminalId}
                  />
                ))}
              </div>
            )}
          </div>
          <div className="tab-pane" style={{ display: tab === "changes" ? "flex" : "none" }}>
            <ChangesView />
          </div>
          <div className="tab-pane" style={{ display: tab === "preview" ? "flex" : "none" }}>
            <Preview />
          </div>
        </main>

        <aside className="panel right">
          {tab === "changes" ? (
            <FileInspector />
          ) : (
            <>
              <WorkbenchTabs active={workbenchTab} onChange={setWorkbenchTab} />
              {workbenchTab === "skills" ? <SkillsPanel /> : <PermissionsPanel />}
            </>
          )}
        </aside>
      </div>
      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}
      <UpdateBanner />
      <ApprovalModal />
    </>
  );
}
