import { useEffect, useRef, useState } from "react";

import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { useModelStore, MODEL_PRESETS, modelLabel } from "../stores/modelStore";
import type { TermInputDetail } from "./Terminal";
import type { WorkspaceContext } from "../types/domain";

export function StatusBar({ workspace }: { workspace?: WorkspaceContext | null }) {
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
      {activeSession && <ModelPill session={activeSession} projectRoot={project.root} />}
      <span className="sb-item sb-muted" title="Live PTY sessions">
        {liveCount} live
      </span>
      {project.language && (
        <span className="sb-item sb-muted" title="Language">{project.language}</span>
      )}
      {project.framework && (
        <span className="sb-item sb-muted" title="Framework">{project.framework}</span>
      )}
      {workspace?.fileCount ? (
        <span className="sb-item sb-muted" title="Tracked files">{workspace.fileCount} files</span>
      ) : null}
      {appVersion && (
        <span className="sb-item sb-muted" title="Agent Console version">v{appVersion}</span>
      )}
    </footer>
  );
}

/// Active-session model indicator + hot-switcher. Reflects the *last requested*
/// model (we can't read Claude's actual loaded model). Picking a model updates
/// the session (so a later resume relaunches with it) and, for a live session,
/// sends `/model <alias>` into the PTY — best-effort: it only takes effect if
/// Claude is idle at its prompt.
function ModelPill({ session, projectRoot }: { session: TerminalSession; projectRoot: string }) {
  const setModel = useTerminalsStore((s) => s.setModel);
  const setDefaultFor = useModelStore((s) => s.setDefaultFor);
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const pick = (model: string) => {
    setOpen(false);
    if (session.model === model) return;
    setModel(session.id, model);
    setDefaultFor(projectRoot, model);
    if (session.status === "live") {
      const detail: TermInputDetail = { sessionId: session.id, data: `/model ${model}\r` };
      window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
    }
  };

  const live = session.status === "live";
  return (
    <div className="model-pill-wrap" ref={wrapRef}>
      <button
        className="sb-item sb-clickable"
        onClick={() => setOpen((v) => !v)}
        title={
          live
            ? "Switch model — sends /model to Claude (works when it's idle at the prompt)"
            : "Model used when this session resumes"
        }
      >
        <span className="model-pill-glyph">◆</span>
        <span>{modelLabel(session.model)}</span>
      </button>
      {open && (
        <div className="model-menu" role="menu">
          <div className="model-menu-head">{live ? "Switch model" : "Model on resume"}</div>
          {MODEL_PRESETS.map((p) => (
            <button
              key={p.model}
              className={`model-menu-item ${session.model === p.model ? "current" : ""}`}
              onClick={() => pick(p.model)}
            >
              <span className="model-menu-icon">{p.icon}</span>
              <span className="model-menu-intent">{p.intent}</span>
              <span className="model-menu-model">{p.label}</span>
            </button>
          ))}
          {live && (
            <div className="model-menu-note">
              Sends <code>/model</code> to the terminal — only takes effect if Claude is idle.
            </div>
          )}
        </div>
      )}
    </div>
  );
}
