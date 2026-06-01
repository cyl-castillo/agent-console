import { useEffect, useRef, useState } from "react";

import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { useModelStore, modelLabel } from "../stores/modelStore";
import { profileFor } from "../agents/profiles";
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
/// model/tuning (we can't read the agent's actually-loaded value). Picking a
/// value updates the session (so a later resume relaunches with it) and, for a
/// live session whose agent supports it (Claude via `/model`), pushes the change
/// into the PTY — best-effort: it only takes effect if the agent is idle at its
/// prompt. Agents without a live switch (Codex) only apply the choice on resume.
function ModelPill({ session, projectRoot }: { session: TerminalSession; projectRoot: string }) {
  const setModel = useTerminalsStore((s) => s.setModel);
  const setDefaultFor = useModelStore((s) => s.setDefaultFor);
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  const profile = profileFor(session.agent);

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
    setDefaultFor(projectRoot, profile.kind, model);
    if (session.status === "live" && profile.supportsLiveModelSwitch && profile.liveModelSwitchInput) {
      const detail: TermInputDetail = { sessionId: session.id, data: profile.liveModelSwitchInput(model) };
      window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
    }
  };

  const live = session.status === "live";
  const canLiveSwitch = live && profile.supportsLiveModelSwitch;
  return (
    <div className="model-pill-wrap" ref={wrapRef}>
      <button
        className="sb-item sb-clickable"
        onClick={() => setOpen((v) => !v)}
        title={
          canLiveSwitch
            ? `Switch model — sends /model to ${profile.label} (works when it's idle at the prompt)`
            : "Model used when this session resumes"
        }
      >
        <span className="model-pill-glyph">{profile.icon}</span>
        <span>{modelLabel(session.model, profile.kind)}</span>
      </button>
      {open && (
        <div className="model-menu" role="menu">
          <div className="model-menu-head">{canLiveSwitch ? "Switch model" : "Model on resume"}</div>
          {profile.models.map((p) => (
            <button
              key={p.value}
              className={`model-menu-item ${session.model === p.value ? "current" : ""}`}
              onClick={() => pick(p.value)}
            >
              <span className="model-menu-icon">{p.icon}</span>
              <span className="model-menu-intent">{p.intent}</span>
              <span className="model-menu-model">{p.label}</span>
            </button>
          ))}
          {canLiveSwitch && (
            <div className="model-menu-note">
              Sends <code>/model</code> to the terminal — only takes effect if {profile.label} is idle.
            </div>
          )}
          {live && !profile.supportsLiveModelSwitch && (
            <div className="model-menu-note">
              Applies the next time this session is resumed.
            </div>
          )}
        </div>
      )}
    </div>
  );
}
