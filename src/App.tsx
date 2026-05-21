import { useEffect } from "react";

import { useSessionStore } from "./stores/sessionStore";
import { attachChatListeners, useChatStore } from "./stores/chatStore";
import { useChangesStore } from "./stores/changesStore";
import { usePreviewStore } from "./stores/previewStore";
import { useUIStore } from "./stores/uiStore";
import { ProjectPicker } from "./components/ProjectPicker";
import { FileTree } from "./components/FileTree";
import { Terminal } from "./components/Terminal";
import { ChangesView } from "./components/ChangesView";
import { AgentChat } from "./components/AgentChat";
import { Preview } from "./components/Preview";
import { PermissionModal } from "./components/PermissionModal";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";

export default function App() {
  const { project, tree, closeProject } = useSessionStore();
  const tab = useUIStore((s) => s.tab);
  const setTab = useUIStore((s) => s.setTab);
  const changesCount = useChangesStore((s) => s.status?.changes.length ?? 0);
  const refreshChanges = useChangesStore((s) => s.refresh);
  const clearChanges = useChangesStore((s) => s.clear);
  const autoSwitchSignal = useChatStore((s) => s.autoSwitchSignal);
  const clearPreview = usePreviewStore((s) => s.clear);

  useKeyboardShortcuts({ setTab });

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    attachChatListeners().then((u) => { unlisten = u; });
    return () => { unlisten?.(); };
  }, []);

  useEffect(() => {
    if (project) refreshChanges();
    else { clearChanges(); clearPreview(); }
  }, [project, refreshChanges, clearChanges, clearPreview]);

  useEffect(() => {
    if (autoSwitchSignal > 0) setTab("changes");
  }, [autoSwitchSignal, setTab]);

  if (!project) {
    return (
      <>
        <ProjectPicker />
        <PermissionModal />
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
          </span>
          <span className="meta" style={{ opacity: 0.6 }}>{project.root}</span>
          <span className="spacer" />
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
          <div className="panel-header">Agent</div>
          <AgentChat />
        </aside>
      </div>
      <PermissionModal />
    </>
  );
}
