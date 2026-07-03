import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from "react";

import { useSessionStore } from "./stores/sessionStore";
import { attachGitWatcherListener, useChangesStore } from "./stores/changesStore";
import { usePreviewStore } from "./stores/previewStore";
import { useUIStore } from "./stores/uiStore";
import { attachSkillsListeners, useSkillsStore } from "./stores/skillsStore";
import { attachSchedulerListeners, useSchedulerStore } from "./stores/schedulerStore";
import { attachApprovalListener } from "./stores/approvalStore";
import { attachVoiceListeners, attachVoiceApprovalWatcher } from "./stores/voiceStore";
import { useUpdaterStore } from "./stores/updaterStore";
import { useTerminalsStore } from "./stores/terminalsStore";
import { ProjectPicker } from "./components/ProjectPicker";
import { LeftSidebar } from "./components/LeftSidebar";
import { useRoundtableStore } from "./stores/roundtableStore";
import { Terminal } from "./components/Terminal";
import { WorktreeBar } from "./components/WorktreeBar";
import { ChangesView } from "./components/ChangesView";
import { Preview } from "./components/Preview";
import { SkillsPanel } from "./components/SkillsPanel";
import { PermissionsPanel } from "./components/PermissionsPanel";
import { AdvisorPanel } from "./components/AdvisorPanel";
import { LearningPanel } from "./components/LearningPanel";
import { RoundtablePanel } from "./components/RoundtablePanel";
import { SchedulerPanel } from "./components/SchedulerPanel";
import { VaultPanel } from "./components/VaultPanel";
import { ContextPanel } from "./components/ContextPanel";
import { FeedbackPanel } from "./components/FeedbackPanel";
import { PluginsPanel } from "./components/PluginsPanel";
import { McpPanel } from "./components/McpPanel";
import { ExportImportPanel } from "./components/ExportImportPanel";
import { useFeedbackStore } from "./stores/feedbackStore";
import { WorkbenchTabs, type WorkbenchTab } from "./components/WorkbenchTabs";
import { ApprovalModal } from "./components/ApprovalModal";
import { FileInspector } from "./components/FileInspector";
import { AboutModal } from "./components/AboutModal";
import { GettingStartedModal } from "./components/GettingStartedModal";
import { OnboardingBanner } from "./components/OnboardingBanner";
import { UpdateBanner } from "./components/UpdateBanner";
import { CommandPalette } from "./components/CommandPalette";
import { StatusBar } from "./components/StatusBar";
import { ShortcutsModal } from "./components/ShortcutsModal";
import { Toasts } from "./components/Toasts";
import { useThemeStore } from "./stores/themeStore";
import { Icon } from "./components/Icon";
import { usePaletteStore } from "./stores/paletteStore";
import { useOnboardingStore } from "./stores/onboardingStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useVoicePtt } from "./hooks/useVoicePtt";
import { useToastStore } from "./stores/toastStore";
import { ipc } from "./ipc/tauri";
import type { WorkspaceContext } from "./types/domain";

const PANEL_W_KEY = "agent-console:panel-width:";
const DEFAULT_W = { left: 240, right: 320 } as const;

function loadPanelWidth(side: "left" | "right"): number {
  try {
    const v = Number(localStorage.getItem(PANEL_W_KEY + side));
    return Number.isFinite(v) && v > 0 ? v : DEFAULT_W[side];
  } catch { return DEFAULT_W[side]; }
}
function savePanelWidth(side: "left" | "right", w: number) {
  try { localStorage.setItem(PANEL_W_KEY + side, String(w)); } catch { /* ignore */ }
}
function clampW(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, Math.round(v)));
}

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
  const [showShortcuts, setShowShortcuts] = useState(false);
  const seenWelcome = useOnboardingStore((s) => s.seenWelcome);
  const markVisitedPermissions = useOnboardingStore((s) => s.markVisitedPermissions);
  type WbTab = WorkbenchTab;
  const [workbenchTab, setWorkbenchTabState] = useState<WbTab>("skills");
  const setWorkbenchTab = (t: WbTab) => {
    setWorkbenchTabState(t);
    if (project) {
      try { localStorage.setItem(`agent-console:workbench-tab:${project.root}`, t); } catch { /* ignore */ }
    }
  };
  const openRoom = useRoundtableStore((s) => s.openRoom);
  // Open a saved room read-only and surface the room tab so it's visible.
  const onOpenRoom = (id: string) => { void openRoom(id); setWorkbenchTab("roundtable"); };
  const initFeedback = useFeedbackStore((s) => s.init);
  const theme = useThemeStore((s) => s.theme);
  const toggleTheme = useThemeStore((s) => s.toggle);
  const [leftOpen, setLeftOpen] = useState(false);
  const checkForUpdates = useUpdaterStore((s) => s.check);
  const resetPaletteForProject = usePaletteStore((s) => s.resetForProject);
  const showToast = useToastStore((s) => s.show);

  const newSession = () => {
    if (!project) return;
    addTerminal(project.root);
    setTab("terminal");
    void persistTerminals();
  };

  // Keep the backend's notion of "the checkout being worked on" in sync with
  // the active session: git/snapshot commands and the change watcher follow
  // the session's isolated worktree (or the project root). Refresh Changes
  // after the switch so the view never shows the previous checkout's status.
  const activeWorktreePath =
    terminalSessions.find((s) => s.id === activeTerminalId)?.worktree?.path ?? null;
  useEffect(() => {
    if (!project) return;
    ipc.setActiveRepo(activeWorktreePath)
      .then(() => refreshChanges())
      .catch(() => { /* stale session referencing a removed worktree — root stays */ });
  }, [project, activeWorktreePath, refreshChanges]);

  const copyProjectPath = () => {
    if (!project) return;
    navigator.clipboard?.writeText(project.root)
      .then(() => showToast("Project path copied", "success"))
      .catch(() => showToast("Could not copy project path", "error"));
  };

  // --- Resizable sidebars ---------------------------------------------------
  const [rightOpen, setRightOpen] = useState(true);
  const [leftW, setLeftW] = useState<number>(() => loadPanelWidth("left"));
  const [rightW, setRightW] = useState<number>(() => loadPanelWidth("right"));
  const appRef = useRef<HTMLDivElement>(null);
  const dragSide = useRef<null | "left" | "right">(null);
  const pendingW = useRef<number | null>(null);

  const startResize = (side: "left" | "right") => (e: ReactPointerEvent) => {
    e.preventDefault();
    dragSide.current = side;
    pendingW.current = null;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
  };
  const onResizeMove = (e: ReactPointerEvent) => {
    const side = dragSide.current;
    const app = appRef.current;
    if (!side || !app) return;
    const rect = app.getBoundingClientRect();
    const cap = Math.round(rect.width * 0.45); // keep the center usable
    let w: number;
    if (side === "left") {
      w = clampW(e.clientX - rect.left, 180, Math.min(480, cap));
      app.style.setProperty("--left-w", `${w}px`);
    } else {
      w = clampW(rect.right - e.clientX, 240, Math.min(600, cap));
      app.style.setProperty("--right-w", `${w}px`);
    }
    pendingW.current = w; // committed to React state on pointer-up
  };
  const endResize = (e: ReactPointerEvent) => {
    const side = dragSide.current;
    if (!side) return;
    const el = e.currentTarget as HTMLElement;
    el.classList.remove("dragging");
    el.releasePointerCapture?.(e.pointerId);
    dragSide.current = null;
    const w = pendingW.current;
    pendingW.current = null;
    if (w == null) return;
    if (side === "left") { setLeftW(w); savePanelWidth("left", w); }
    else { setRightW(w); savePanelWidth("right", w); }
  };
  const resetResize = (side: "left" | "right") => () => {
    const w = DEFAULT_W[side];
    if (side === "left") setLeftW(w); else setRightW(w);
    savePanelWidth(side, w);
  };

  useKeyboardShortcuts({ setTab });
  useVoicePtt();

  // Listen for palette-triggered navigation events.
  useEffect(() => {
    const onOpenTab = (e: Event) => {
      const d = (e as CustomEvent).detail;
      if (d === "terminal" || d === "changes" || d === "preview") setTab(d);
    };
    const onOpenWb = (e: Event) => {
      const d = (e as CustomEvent).detail;
      if (d === "skills" || d === "permissions" || d === "advisor" || d === "learning" || d === "schedule" || d === "vault" || d === "context" || d === "plugins" || d === "mcp" || d === "transfer" || d === "feedback") {
        setWorkbenchTab(d);
      }
    };
    const onGettingStarted = () => setShowGettingStarted(true);
    const onShortcuts = () => setShowShortcuts(true);
    const onToggleSidebar = () => setLeftOpen((v) => !v);
    const onToggleRight = () => setRightOpen((v) => !v);
    const onNewSession = () => newSession();
    const onCopyProjectPath = () => copyProjectPath();
    window.addEventListener("ac:open-tab", onOpenTab);
    window.addEventListener("ac:open-workbench-tab", onOpenWb);
    window.addEventListener("ac:open-getting-started", onGettingStarted);
    window.addEventListener("ac:open-shortcuts", onShortcuts);
    window.addEventListener("ac:toggle-sidebar", onToggleSidebar);
    window.addEventListener("ac:toggle-right-panel", onToggleRight);
    window.addEventListener("ac:new-session", onNewSession);
    window.addEventListener("ac:copy-project-path", onCopyProjectPath);
    return () => {
      window.removeEventListener("ac:open-tab", onOpenTab);
      window.removeEventListener("ac:open-workbench-tab", onOpenWb);
      window.removeEventListener("ac:open-getting-started", onGettingStarted);
      window.removeEventListener("ac:open-shortcuts", onShortcuts);
      window.removeEventListener("ac:toggle-sidebar", onToggleSidebar);
      window.removeEventListener("ac:toggle-right-panel", onToggleRight);
      window.removeEventListener("ac:new-session", onNewSession);
      window.removeEventListener("ac:copy-project-path", onCopyProjectPath);
    };
  }, [project, setTab, addTerminal, persistTerminals, showToast]);

  useEffect(() => {
    // If cleanup runs before a listen() promise resolves (StrictMode double
    // mount, fast unmount), unlisten immediately instead of leaking it.
    let disposed = false;
    let offSkills: (() => void) | null = null;
    let offApproval: (() => void) | null = null;
    let offGit: (() => void) | null = null;
    let offVoice: (() => void) | null = null;
    let offScheduler: (() => void) | null = null;
    attachSkillsListeners().then((u) => { if (disposed) u(); else offSkills = u; });
    attachApprovalListener().then((u) => { if (disposed) u(); else offApproval = u; });
    attachGitWatcherListener().then((u) => { if (disposed) u(); else offGit = u; });
    attachVoiceListeners().then((u) => { if (disposed) u(); else offVoice = u; });
    attachSchedulerListeners().then((u) => { if (disposed) u(); else offScheduler = u; });
    const offVoiceApproval = attachVoiceApprovalWatcher();
    return () => { disposed = true; offSkills?.(); offApproval?.(); offGit?.(); offVoice?.(); offScheduler?.(); offVoiceApproval(); };
  }, []);

  useEffect(() => {
    checkForUpdates({ silentIfNone: true });
    // Re-check periodically: this app commonly stays open for days, so a
    // startup-only check means a long-running session never learns about a new
    // release. Silent unless something's actually available.
    const id = window.setInterval(
      () => checkForUpdates({ silentIfNone: true }),
      6 * 60 * 60 * 1000,
    );
    return () => window.clearInterval(id);
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
    void useSchedulerStore.getState().refresh();
    resetPaletteForProject(project.root);
    // Palette file index is built lazily on first palette open (see
    // paletteStore.openPalette) — not eagerly here, to keep the full-tree
    // walk off the project-open path.
    // Restore last workbench tab for this project, if any.
    try {
      const saved = localStorage.getItem(`agent-console:workbench-tab:${project.root}`);
      if (saved === "skills" || saved === "permissions" || saved === "advisor"
          || saved === "learning" || saved === "roundtable" || saved === "schedule" || saved === "vault" || saved === "context"
          || saved === "plugins" || saved === "mcp" || saved === "transfer" || saved === "feedback") {
        setWorkbenchTabState(saved);
      }
    } catch { /* ignore */ }
    ipc.workspaceContext().then(setWorkspace).catch(() => setWorkspace(null));
    (async () => {
      await hydrateTerminals(project.root);
      // Auto-spawn one live session only when the load SUCCEEDED and restored
      // nothing. If hydrate failed (ready=false) we must not spawn — doing so
      // would let the next persist overwrite history we simply couldn't read.
      const st = useTerminalsStore.getState();
      if (st.ready && st.sessions.length === 0) {
        addTerminal(project.root);
      }
      // Clean up managed worktree checkouts no session references any more
      // (branches are kept). Only when hydrate succeeded — a failed load must
      // not be allowed to orphan every live worktree.
      if (st.ready) {
        const keep = st.sessions
          .map((s) => s.worktree?.path)
          .filter((p): p is string => Boolean(p));
        ipc.worktreePruneOrphans(keep)
          .then((removed) => {
            if (removed.length > 0) {
              showToast(`Cleaned ${removed.length} orphaned worktree${removed.length === 1 ? "" : "s"} (branches kept)`, "info");
            }
          })
          .catch(() => { /* best-effort */ });
      }
    })();
  }, [project, refreshChanges, refreshSkills, clearChanges, clearPreview, hydrateTerminals, clearTerminals, addTerminal, resetPaletteForProject, showToast]);

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
        {showShortcuts && <ShortcutsModal onClose={() => setShowShortcuts(false)} />}
        <UpdateBanner />
        <Toasts />
      </>
    );
  }

  return (
    <>
      <div
        ref={appRef}
        className={`app ${leftOpen ? "" : "left-collapsed"} ${rightOpen ? "" : "right-collapsed"}`}
        style={{ "--left-w": `${leftW}px`, "--right-w": `${rightW}px` } as CSSProperties}
      >
        <div className="topbar">
          <button
            className={`sidebar-toggle ${leftOpen ? "open" : ""}`}
            onClick={() => setLeftOpen((v) => !v)}
            title={leftOpen ? "Hide workspace" : "Show workspace"}
          >
            <span className="sidebar-toggle-icon"><Icon name="panel-left" size={14} /></span>
            <span>Workspace</span>
            {changesCount > 0 && <span className="sidebar-toggle-badge">{changesCount}</span>}
          </button>

          <button
            className="tb-project"
            onClick={copyProjectPath}
            title={`${project.root}\n(click to copy path)`}
          >
            <span className="tb-project-name">{project.name}</span>
          </button>

          <span className="spacer" />

          <button
            className={`topbar-icon ${rightOpen ? "on" : ""}`}
            onClick={() => setRightOpen((v) => !v)}
            title={rightOpen ? "Hide side panel (Ctrl+J)" : "Show side panel (Ctrl+J)"}
          >
            <Icon name="panel-right" size={14} />
          </button>
          <button
            className="topbar-icon"
            onClick={toggleTheme}
            title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          >
            <Icon name={theme === "dark" ? "sun" : "moon"} size={14} />
          </button>
          <button
            className="topbar-icon"
            onClick={() => setShowShortcuts(true)}
            title="Keyboard shortcuts (Ctrl+/)"
            aria-label="Keyboard shortcuts"
          ><Icon name="command" size={14} /></button>
          <button
            className="topbar-icon"
            onClick={() => setShowGettingStarted(true)}
            title="Getting started guide"
            aria-label="Getting started guide"
          ><Icon name="help-circle" size={14} /></button>
          <button
            className="topbar-icon"
            onClick={() => setShowAbout(true)}
            title="About Agent Console"
            aria-label="About Agent Console"
          ><Icon name="info" size={14} /></button>
          <button
            className="tb-close"
            onClick={() => { persistTerminals(); closeProject(); }}
            title="Close project"
          >Close</button>
        </div>

        {leftOpen && <LeftSidebar onOpenRoom={onOpenRoom} />}

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
              <div className="terminal-empty">
                <div>
                  <div className="terminal-empty-title">No active sessions</div>
                  <div className="terminal-empty-copy">Each new session starts in this project and launches its agent automatically.</div>
                </div>
                <button className="btn btn-primary" onClick={newSession}>+ New session</button>
              </div>
            ) : (
              <>
                <WorktreeBar />
                <div className="terminals-stack">
                  {liveTerminals.map((s) => (
                    <Terminal
                      key={s.id}
                      session={s}
                      visible={tab === "terminal" && s.id === activeTerminalId}
                    />
                  ))}
                </div>
              </>
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
                {workbenchTab === "learning" && <LearningPanel />}
                {workbenchTab === "roundtable" && <RoundtablePanel />}
                {workbenchTab === "schedule" && <SchedulerPanel />}
                {workbenchTab === "vault" && <VaultPanel />}
                {workbenchTab === "context" && <ContextPanel />}
                {workbenchTab === "plugins" && <PluginsPanel />}
                {workbenchTab === "mcp" && <McpPanel />}
                {workbenchTab === "transfer" && <ExportImportPanel />}
                {workbenchTab === "feedback" && <FeedbackPanel />}
              </div>
            </div>
          )}
        </aside>
        <StatusBar workspace={workspace} />

        {leftOpen && (
          <div
            className="resize-handle left"
            onPointerDown={startResize("left")}
            onPointerMove={onResizeMove}
            onPointerUp={endResize}
            onDoubleClick={resetResize("left")}
            title="Drag to resize · double-click to reset"
          />
        )}
        {rightOpen && (
          <div
            className="resize-handle right"
            onPointerDown={startResize("right")}
            onPointerMove={onResizeMove}
            onPointerUp={endResize}
            onDoubleClick={resetResize("right")}
            title="Drag to resize · double-click to reset"
          />
        )}
      </div>
      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}
      {showGettingStarted && (
        <GettingStartedModal
          onClose={() => setShowGettingStarted(false)}
          onJumpToTab={(t) => { setWorkbenchTab(t); }}
        />
      )}
      {showShortcuts && <ShortcutsModal onClose={() => setShowShortcuts(false)} />}
      <OnboardingBanner onOpen={() => setShowGettingStarted(true)} />
      <UpdateBanner />
      <ApprovalModal />
      <CommandPalette />
      <Toasts />
    </>
  );
}
