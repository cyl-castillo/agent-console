import { useEffect, useState } from "react";

import { useSessionStore } from "./stores/sessionStore";
import { attachGitWatcherListener, useChangesStore } from "./stores/changesStore";
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
import { AdvisorPanel } from "./components/AdvisorPanel";
import { WorkbenchTabs } from "./components/WorkbenchTabs";
import { ApprovalModal } from "./components/ApprovalModal";
import { FileInspector } from "./components/FileInspector";
import { AboutModal } from "./components/AboutModal";
import { GettingStartedModal } from "./components/GettingStartedModal";
import { OnboardingBanner } from "./components/OnboardingBanner";
import { UpdateBanner } from "./components/UpdateBanner";
import { useOnboardingStore } from "./stores/onboardingStore";
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
  const [showGettingStarted, setShowGettingStarted] = useState(false);
  const seenWelcome = useOnboardingStore((s) => s.seenWelcome);
  const markVisitedPermissions = useOnboardingStore((s) => s.markVisitedPermissions);
  const [workbenchTab, setWorkbenchTab] = useState<"skills" | "permissions" | "advisor">("skills");
  const [leftOpen, setLeftOpen] = useState(false);
  const checkForUpdates = useUpdaterStore((s) => s.check);

  useKeyboardShortcuts({ setTab });

  useEffect(() => {
    let offSkills: (() => void) | null = null;
    let offApproval: (() => void) | null = null;
    let offGit: (() => void) | null = null;
    attachSkillsListeners().then((u) => { offSkills = u; });
    attachApprovalListener().then((u) => { offApproval = u; });
    attachGitWatcherListener().then((u) => { offGit = u; });
    return () => { offSkills?.(); offApproval?.(); offGit?.(); };
  }, []);

  useEffect(() => {
    checkForUpdates({ silentIfNone: true });
  }, [checkForUpdates]);

  // First-time auto-open of the Getting Started guide.
  useEffect(() => {
    if (!seenWelcome) setShowGettingStarted(true);
  }, [seenWelcome]);

  // Mark permissions tab as visited as soon as the user opens it.
  useEffect(() => {
    if (workbenchTab === "permissions") markVisitedPermissions();
  }, [workbenchTab, markVisitedPermissions]);

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
        {showGettingStarted && (
          <GettingStartedModal
            onClose={() => setShowGettingStarted(false)}
            onJumpToTab={setWorkbenchTab}
          />
        )}
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

          <button
            className="tb-project"
            onClick={() => { navigator.clipboard?.writeText(project.root).catch(() => {}); }}
            title={`${project.root}\n(click to copy path)`}
          >
            <span className="tb-project-name">{project.name}</span>
          </button>

          <div className="tb-pills">
            {project.language && (
              <span className="tb-pill tb-pill-lang" title="Language">{project.language}</span>
            )}
            {project.framework && (
              <span className="tb-pill" title="Framework">{project.framework}</span>
            )}
            {branch && (
              <span className="tb-pill tb-pill-branch" title="Git branch">
                <span className="tb-pill-icon">⎇</span>{branch}
              </span>
            )}
            {workspace?.fileCount ? (
              <span className="tb-pill tb-pill-files" title="Tracked files">
                {workspace.fileCount} files
              </span>
            ) : null}
          </div>

          <span className="spacer" />

          <button
            className="topbar-icon"
            onClick={() => setShowGettingStarted(true)}
            title="Getting started guide"
          >?</button>
          <button
            className="topbar-icon"
            onClick={() => setShowAbout(true)}
            title="About Agent Console"
          >ⓘ</button>
          <button
            className="tb-close"
            onClick={() => { persistTerminals(); closeProject(); }}
            title="Close project"
          >Close</button>
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
                No hay sesiones activas. Abrí el sidebar (botón <strong>Workspace</strong>)
                y dale <strong>+ new</strong> en Sessions. Cada terminal nueva
                auto-ejecuta <code>claude</code>.
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
              {workbenchTab === "skills" && <SkillsPanel />}
              {workbenchTab === "permissions" && <PermissionsPanel />}
              {workbenchTab === "advisor" && <AdvisorPanel />}
            </>
          )}
        </aside>
      </div>
      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}
      {showGettingStarted && (
        <GettingStartedModal
          onClose={() => setShowGettingStarted(false)}
          onJumpToTab={(t) => { setWorkbenchTab(t); }}
        />
      )}
      <OnboardingBanner onOpen={() => setShowGettingStarted(true)} />
      <UpdateBanner />
      <ApprovalModal />
    </>
  );
}
