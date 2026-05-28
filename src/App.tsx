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
import { VaultPanel } from "./components/VaultPanel";
import { ContextPanel } from "./components/ContextPanel";
import { FeedbackPanel } from "./components/FeedbackPanel";
import { useFeedbackStore } from "./stores/feedbackStore";
import { WorkbenchTabs } from "./components/WorkbenchTabs";
import { ApprovalModal } from "./components/ApprovalModal";
import { FileInspector } from "./components/FileInspector";
import { AboutModal } from "./components/AboutModal";
import { GettingStartedModal } from "./components/GettingStartedModal";
import { OnboardingBanner } from "./components/OnboardingBanner";
import { UpdateBanner } from "./components/UpdateBanner";
import { CommandPalette } from "./components/CommandPalette";
import { StatusBar } from "./components/StatusBar";
import { usePaletteStore } from "./stores/paletteStore";
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
  const [workbenchTab, setWorkbenchTab] = useState<"skills" | "permissions" | "advisor" | "vault" | "context" | "feedback">("skills");
  const initFeedback = useFeedbackStore((s) => s.init);
  const [leftOpen, setLeftOpen] = useState(false);
  const checkForUpdates = useUpdaterStore((s) => s.check);
  const reloadPaletteIndex = usePaletteStore((s) => s.reloadIndex);
  const resetPaletteForProject = usePaletteStore((s) => s.resetForProject);

  useKeyboardShortcuts({ setTab });

  // Listen for palette-triggered navigation events.
  useEffect(() => {
    const onOpenTab = (e: Event) => {
      const d = (e as CustomEvent).detail;
      if (d === "terminal" || d === "changes" || d === "preview") setTab(d);
    };
    const onOpenWb = (e: Event) => {
      const d = (e as CustomEvent).detail;
      if (d === "skills" || d === "permissions" || d === "advisor" || d === "vault" || d === "context") {
        setWorkbenchTab(d);
      }
    };
    const onGettingStarted = () => setShowGettingStarted(true);
    window.addEventListener("ac:open-tab", onOpenTab);
    window.addEventListener("ac:open-workbench-tab", onOpenWb);
    window.addEventListener("ac:open-getting-started", onGettingStarted);
    return () => {
      window.removeEventListener("ac:open-tab", onOpenTab);
      window.removeEventListener("ac:open-workbench-tab", onOpenWb);
      window.removeEventListener("ac:open-getting-started", onGettingStarted);
    };
  }, [setTab]);

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

  useEffect(() => { void initFeedback(); }, [initFeedback]);

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
      resetPaletteForProject(null);
      return;
    }
    refreshChanges();
    refreshSkills();
    resetPaletteForProject(project.root);
    void reloadPaletteIndex();
    ipc.workspaceContext().then(setWorkspace).catch(() => setWorkspace(null));
    (async () => {
      await hydrateTerminals(project.root);
      // Auto-spawn one live session if nothing was restored from disk.
      if (useTerminalsStore.getState().sessions.length === 0) {
        addTerminal(project.root);
      }
    })();
  }, [project, refreshChanges, refreshSkills, clearChanges, clearPreview, hydrateTerminals, clearTerminals, addTerminal, reloadPaletteIndex, resetPaletteForProject]);

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
              <span>Terminal</span>
              <span className="tab-shortcut">Ctrl 1</span>
            </button>
            <button
              className={`tab ${tab === "changes" ? "active" : ""}`}
              onClick={() => setTab("changes")}
              title="Ctrl+2"
            >
              <span>Changes</span>
              {changesCount > 0 && <span className="tab-badge">{changesCount}</span>}
              <span className="tab-shortcut">Ctrl 2</span>
            </button>
            <button
              className={`tab ${tab === "preview" ? "active" : ""}`}
              onClick={() => setTab("preview")}
              title="Ctrl+3"
            >
              <span>Preview</span>
              <span className="tab-shortcut">Ctrl 3</span>
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
            <div className="workbench-layout">
              <WorkbenchTabs active={workbenchTab} onChange={setWorkbenchTab} />
              <div className="workbench-content">
                {workbenchTab === "skills" && <SkillsPanel />}
                {workbenchTab === "permissions" && <PermissionsPanel />}
                {workbenchTab === "advisor" && <AdvisorPanel />}
                {workbenchTab === "vault" && <VaultPanel />}
                {workbenchTab === "context" && <ContextPanel />}
                {workbenchTab === "feedback" && <FeedbackPanel />}
              </div>
            </div>
          )}
        </aside>
        <StatusBar workspace={workspace} />
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
      <CommandPalette />
    </>
  );
}
