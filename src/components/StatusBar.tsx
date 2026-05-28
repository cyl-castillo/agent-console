import { useEffect } from "react";

import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";

export function StatusBar() {
  const project = useSessionStore((s) => s.project);
  const status = useChangesStore((s) => s.status);
  const branches = useChangesStore((s) => s.branches);
  const setTab = useUIStore((s) => s.setTab);
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const loadBranches = useChangesStore((s) => s.loadBranches);
  const branch = status?.branch ?? null;

  // Lazy-load branches once we know the current branch, so ahead/behind shows.
  useEffect(() => {
    if (branch && branches.length === 0) void loadBranches();
  }, [branch, branches.length, loadBranches]);

  if (!project) return null;

  const branchInfo = branches.find((b) => b.name === branch);
  const ahead = branchInfo?.ahead ?? 0;
  const behind = branchInfo?.behind ?? 0;
  const changesCount = status?.changes.length ?? 0;
  const liveCount = sessions.filter((s) => s.status === "live").length;
  const activeSession = sessions.find((s) => s.id === activeId);
  const appVersion = __APP_VERSION__;

  return (
    <footer className="statusbar">
      {branch && (
        <button
          className="sb-item sb-clickable"
          onClick={() => setTab("changes")}
          title="Open Changes"
        >
          <span className="sb-icon">⎇</span>
          <span>{branch}</span>
          {(ahead > 0 || behind > 0) && (
            <span className="sb-sub">
              {ahead > 0 && <>↑{ahead}</>}
              {behind > 0 && <>↓{behind}</>}
            </span>
          )}
        </button>
      )}
      {status?.isRepo && (
        <button
          className={`sb-item sb-clickable ${changesCount > 0 ? "sb-warn" : ""}`}
          onClick={() => setTab("changes")}
          title="Open Changes"
        >
          <span>{changesCount} change{changesCount === 1 ? "" : "s"}</span>
        </button>
      )}
      <div className="sb-spacer" />
      {activeSession && (
        <button
          className="sb-item sb-clickable"
          onClick={() => setTab("terminal")}
          title="Open Terminal"
        >
          <span className={`sb-dot sb-dot-${activeSession.status}`} />
          <span>{activeSession.name}</span>
        </button>
      )}
      <span className="sb-item sb-muted" title="Live PTY sessions">
        {liveCount} live
      </span>
      {project.language && (
        <span className="sb-item sb-muted" title="Language">{project.language}</span>
      )}
      {appVersion && (
        <span className="sb-item sb-muted" title="Agent Console version">v{appVersion}</span>
      )}
    </footer>
  );
}
