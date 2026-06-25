import { useEffect, useRef, useState } from "react";

import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { useModelStore, modelLabel } from "../stores/modelStore";
import { useVoiceStore } from "../stores/voiceStore";
import { useApprovalStore } from "../stores/approvalStore";
import { useAgentStatusStore } from "../stores/agentStatusStore";
import { profileFor } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import type { TermInputDetail } from "./Terminal";
import type { SessionUsage, WorkspaceContext } from "../types/domain";

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
      <AgentStatePill onShowTerminal={() => setTab("terminal")} />
      {activeSession && <ModelPill session={activeSession} projectRoot={project.root} />}
      {activeSession?.claudeSessionId && (
        <UsagePill
          sessionId={activeSession.claudeSessionId}
          projectRoot={project.root}
          live={activeSession.status === "live"}
        />
      )}
      <VoicePill />
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

/// Glanceable agent activity. "waiting on you" (an approval is pending) is the
/// reliable state; "working…" is a best-effort recent-activity hint that decays,
/// since the CLI gives us no turn-completion signal (see agentStatusStore). Idle
/// renders nothing — the live session dot already says the agent is up.
function AgentStatePill({ onShowTerminal }: { onShowTerminal: () => void }) {
  const blocked = useApprovalStore((s) => s.queue.length);
  const workingUntil = useAgentStatusStore((s) => s.workingUntil);
  const [, force] = useState(0);

  // Re-render when the recent-activity window elapses so it falls back to idle.
  useEffect(() => {
    const left = workingUntil - Date.now();
    if (left <= 0) return;
    const t = setTimeout(() => force((n) => n + 1), left + 50);
    return () => clearTimeout(t);
  }, [workingUntil]);

  const state: "blocked" | "working" | "idle" =
    blocked > 0 ? "blocked" : Date.now() < workingUntil ? "working" : "idle";

  if (state === "idle") return null;

  if (state === "blocked") {
    return (
      <button
        className="sb-item sb-clickable sb-agent sb-agent-blocked"
        onClick={onShowTerminal}
        title={`Agent is waiting for you to approve ${blocked} action${blocked === 1 ? "" : "s"}`}
      >
        <span className="sb-agent-dot" />
        <span>waiting on you{blocked > 1 ? ` (${blocked})` : ""}</span>
      </button>
    );
  }

  return (
    <span
      className="sb-item sb-agent sb-agent-working"
      title="The agent was active in the last few seconds. The CLI doesn't report turn completion, so this is best-effort."
    >
      <span className="sb-agent-dot" />
      <span>working…</span>
    </span>
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

/// Local voice input toggle + state. Click (or Ctrl+Shift+V) enables voice
/// mode: first use downloads the Whisper model, then holding Ctrl+Space
/// records and releasing types the transcript into the active composer.
function VoicePill() {
  const phase = useVoiceStore((s) => s.phase);
  const progress = useVoiceStore((s) => s.progress);
  const toggle = useVoiceStore((s) => s.toggle);

  const pct = progress?.total
    ? Math.round((progress.downloaded / progress.total) * 100)
    : null;
  const label =
    phase === "off" ? "voice off"
    : phase === "loading" ? (pct != null ? `voice ${pct}%` : "voice loading…")
    : phase === "listening" ? "listening…"
    : phase === "transcribing" ? "transcribing…"
    : "voice ready";
  const title =
    phase === "off"
      ? "Enable voice input (Ctrl+Shift+V). First use downloads the Whisper model (~190 MB, local)."
      : phase === "loading"
        ? "Downloading / loading the Whisper model…"
        : "Hold Ctrl+Space to talk; release to type into the composer. Click to disable.";

  return (
    <button
      className={`sb-item sb-clickable voice-pill voice-${phase}`}
      onClick={() => void toggle()}
      title={title}
    >
      <span className="voice-glyph">{phase === "listening" ? "●" : "🎙"}</span>
      <span>{label}</span>
    </button>
  );
}

/// Context-usage indicator for the active Claude session. Reads the session
/// transcript via `session_usage` and shows how full the model context is
/// (`contextTokens / contextWindow`). Polls while the session is live so it
/// tracks the agent's progress; the totals live in the tooltip. Turns amber
/// past 80% as a hint to `/compact`.
function UsagePill({ sessionId, projectRoot, live }: { sessionId: string; projectRoot: string; live: boolean }) {
  const [usage, setUsage] = useState<SessionUsage | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      ipc.sessionUsage(sessionId, projectRoot)
        .then((u) => { if (!cancelled) setUsage(u); })
        .catch(() => { /* transcript not ready / unreadable — keep last value */ });
    };
    load();
    // Only poll while the agent can still be producing tokens.
    const t = live ? window.setInterval(load, 5000) : null;
    return () => { cancelled = true; if (t) window.clearInterval(t); };
  }, [sessionId, projectRoot, live]);

  if (!usage || usage.contextTokens <= 0) return null;

  const pct = Math.round((usage.contextTokens / usage.contextWindow) * 100);
  const warn = pct >= 80;
  const tip =
    `Context window: ${fmtTokens(usage.contextTokens)} / ${fmtTokens(usage.contextWindow)} (${pct}%)\n` +
    `Input (cumulative): ${fmtTokens(usage.inputTotal)}\n` +
    `Output (cumulative): ${fmtTokens(usage.outputTotal)}\n` +
    `Cache read: ${fmtTokens(usage.cacheReadTotal)}\n` +
    `Cache write: ${fmtTokens(usage.cacheCreationTotal)}`;

  return (
    <span className={`sb-item sb-muted usage-pill ${warn ? "usage-warn" : ""}`} title={tip}>
      <span className="usage-glyph">⌁</span>
      <span>{fmtTokens(usage.contextTokens)} ({pct}%)</span>
    </span>
  );
}

/// Compact token count: 1234 → "1.2k", 84000 → "84k", 512 → "512".
function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  const k = n / 1000;
  return `${k >= 10 ? Math.round(k) : k.toFixed(1)}k`;
}
