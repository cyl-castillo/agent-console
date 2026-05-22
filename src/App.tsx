import { useEffect, useState } from "react";

import { useSessionStore } from "./stores/sessionStore";
import { useChangesStore } from "./stores/changesStore";
import { usePreviewStore } from "./stores/previewStore";
import { useUIStore } from "./stores/uiStore";
import { attachSkillsListeners, useSkillsStore } from "./stores/skillsStore";
import { useUpdaterStore } from "./stores/updaterStore";
import { ProjectPicker } from "./components/ProjectPicker";
import { FileTree } from "./components/FileTree";
import { Terminal } from "./components/Terminal";
import { ChangesView } from "./components/ChangesView";
import { Preview } from "./components/Preview";
import { SkillsPanel } from "./components/SkillsPanel";
import { AboutModal } from "./components/AboutModal";
import { UpdateBanner } from "./components/UpdateBanner";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { ipc } from "./ipc/tauri";
import type { WorkspaceContext } from "./types/domain";

export default function App() {
  const { project, tree, closeProject } = useSessionStore();
  const tab = useUIStore((s) => s.tab);
  const setTab = useUIStore((s) => s.setTab);
  const changesCount = useChangesStore((s) => s.status?.changes.length ?? 0);
  const refreshChanges = useChangesStore((s) => s.refresh);
  const clearChanges = useChangesStore((s) => s.clear);
  const clearPreview = usePreviewStore((s) => s.clear);
  const refreshSkills = useSkillsStore((s) => s.refresh);
  const branch = useChangesStore((s) => s.status?.branch ?? null);
  const [workspace, setWorkspace] = useState<WorkspaceContext | null>(null);
  const [showAbout, setShowAbout] = useState(false);
  const checkForUpdates = useUpdaterStore((s) => s.check);

  useKeyboardShortcuts({ setTab });

  // Attach hook event bridge once.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    attachSkillsListeners().then((u) => { unlisten = u; });
    return () => { unlisten?.(); };
  }, []);

  // Silent update check on startup.
  useEffect(() => {
    checkForUpdates({ silentIfNone: true });
  }, [checkForUpdates]);

  // Reload git status + skills + workspace when project changes.
  useEffect(() => {
    if (project) {
      refreshChanges();
      refreshSkills();
      ipc.workspaceContext().then(setWorkspace).catch(() => setWorkspace(null));
    } else {
      clearChanges();
      clearPreview();
      setWorkspace(null);
    }
  }, [project, refreshChanges, refreshSkills, clearChanges, clearPreview]);

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
      <div className="app">
        <div className="topbar">
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
          <button onClick={closeProject}>Close</button>
        </div>

        <aside className="panel left">
          <div className="panel-header">Files</div>
          {tree ? <FileTree root={tree} /> : <div className="placeholder">Loading…</div>}
        </aside>

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
            <Terminal cwd={project.root} />
          </div>
          <div className="tab-pane" style={{ display: tab === "changes" ? "flex" : "none" }}>
            <ChangesView />
          </div>
          <div className="tab-pane" style={{ display: tab === "preview" ? "flex" : "none" }}>
            <Preview />
          </div>
        </main>

        <aside className="panel right">
          <div className="panel-header">Workbench</div>
          <SkillsPanel />
        </aside>
      </div>
      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}
      <UpdateBanner />
    </>
  );
}
