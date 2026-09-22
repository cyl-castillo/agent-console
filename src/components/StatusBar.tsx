import { useEffect, useRef, useState } from "react";

import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { useModelStore, modelLabel } from "../stores/modelStore";
import { useVoiceStore } from "../stores/voiceStore";
import { useApprovalStore } from "../stores/approvalStore";
import { useAgentStatusStore } from "../stores/agentStatusStore";
import { useToastStore } from "../stores/toastStore";
import { useSkillsStore } from "../stores/skillsStore";
import { CHECK_INTERVAL_MS, useHooksHealthStore } from "../stores/hooksHealthStore";
import { freshStatus, useLiveStatusStore } from "../stores/liveStatusStore";
import { formatAge } from "../lib/hooksHealth";
import { useInjectStore } from "../stores/injectStore";
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

  // Hooks must run unconditionally — keep this above the early return.
  const build = useBuildInfo();

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
          <span>
            {changesCount} change{changesCount === 1 ? "" : "s"}
          </span>
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
      {activeSession && <MemoryPill termId={activeSession.id} />}
      {activeSession && <ModelPill session={activeSession} projectRoot={project.root} />}
      {activeSession?.agentSessionId && (
        <UsagePill
          termId={activeSession.id}
          sessionId={activeSession.agentSessionId}
          projectRoot={project.root}
          agent={activeSession.agent}
          live={activeSession.status === "live"}
        />
      )}
      <VoicePill />
      <HooksPill />
      <span className="sb-item sb-muted" title="Live PTY sessions">
        {liveCount} live
      </span>
      {project.language && (
        <span className="sb-item sb-muted" title="Language">
          {project.language}
        </span>
      )}
      {project.framework && (
        <span className="sb-item sb-muted" title="Framework">
          {project.framework}
        </span>
      )}
      {workspace?.fileCount ? (
        <span className="sb-item sb-muted" title="Tracked files">
          {workspace.fileCount} files
        </span>
      ) : null}
      {build?.debug && (
        <span
          className="sb-item sb-muted"
          title={`Dev build — commit ${build.commit}, built ${build.builtAt}. If this doesn't match what you just changed, you're on a stale binary.`}
        >
          dev {build.commit} · {build.builtAt}
        </span>
      )}
      {appVersion && (
        <span className="sb-item sb-muted" title="Agent Console version">
          v{appVersion}
        </span>
      )}
    </footer>
  );
}

/// Is the CLI→console bridge alive? Four states, two of them actionable:
/// off (hooks not installed — click installs), silent (installed, nothing
/// seen yet), ok (last event N ago), stale (a live Claude session swallowed
/// prompts with no hook event — click reinstalls; the toast explains trust).
function HooksPill() {
  const verdict = useHooksHealthStore((s) => s.verdict);
  const check = useHooksHealthStore((s) => s.check);
  const install = useSkillsStore((s) => s.install);
  const [, force] = useState(0);

  useEffect(() => {
    void check();
    const t = setInterval(() => {
      void check();
      force((n) => n + 1);
    }, CHECK_INTERVAL_MS);
    return () => clearInterval(t);
  }, [check]);

  const reinstall = async () => {
    await install();
    useToastStore
      .getState()
      .show("Hooks reinstalled. Send a prompt in the terminal to confirm they report.", "info");
    void check();
  };

  switch (verdict.kind) {
    case "off":
      return (
        <button
          className="sb-item sb-clickable sb-warn"
          onClick={() => void reinstall()}
          title="Hooks are not installed: no approval modal, no proof ledger, no snapshots, no session resume. Click to install them (~/.claude/settings.json and, if present, ~/.codex/hooks.json)."
        >
          hooks off
        </button>
      );
    case "silent":
      return (
        <span
          className="sb-item sb-muted"
          title="Hooks are installed. The first prompt you send inside a terminal confirms they report; until then there is nothing to judge."
        >
          hooks · no events yet
        </span>
      );
    case "stale":
      return (
        <button
          className="sb-item sb-clickable sb-agent sb-agent-blocked"
          onClick={() => void reinstall()}
          title={`Prompts were sent in ${verdict.termIds.length} terminal${verdict.termIds.length === 1 ? "" : "s"} with a live Claude session and no hook event arrived. Approvals, proof, snapshots and resume are blind there. Most often the folder isn't trusted by Claude Code (hooks silently skip untrusted directories): run \`claude\` there once and accept the trust prompt. Click to reinstall hooks in case settings were overwritten.`}
        >
          <span className="sb-agent-dot" />
          <span>hooks · not reporting</span>
        </button>
      );
    case "ok":
      return (
        <span
          className="sb-item sb-muted"
          title={`Hooks are reporting. Last event ${formatAge(Date.now() - verdict.lastEventMs)} ago.`}
        >
          hooks · {formatAge(Date.now() - verdict.lastEventMs)}
        </span>
      );
  }
}

/// Format elapsed working time compactly: 42s / 2m 05s / 1h 12m.
function formatWorking(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${(s % 60).toString().padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${(m % 60).toString().padStart(2, "0")}m`;
}

/// Glanceable agent activity. "waiting on you" (an approval is pending) is the
/// reliable state; "working… N" shows elapsed time since the turn started and
/// ends precisely on the Stop hook (activity decay stays as the fallback for
/// hooks not yet installed/trusted). Idle renders nothing — the live session
/// dot already says the agent is up.
/// Build provenance chip data, dev builds only. One fetch per mount; the
/// answer never changes for a running binary — that's the point: if the chip
/// doesn't match what you just edited, you're staring at a stale binary.
function useBuildInfo(): { commit: string; builtAt: string; debug: boolean } | null {
  const [info, setInfo] = useState<{ commit: string; builtAt: string; debug: boolean } | null>(
    null,
  );
  useEffect(() => {
    ipc
      .appBuildInfo()
      .then((b) =>
        setInfo({
          commit: b.commit,
          builtAt: b.buildTimeSecs
            ? new Date(b.buildTimeSecs * 1000).toLocaleTimeString(undefined, {
                hour: "2-digit",
                minute: "2-digit",
              })
            : "?",
          debug: b.debug,
        }),
      )
      .catch(() => setInfo(null));
  }, []);
  return info;
}

function AgentStatePill({ onShowTerminal }: { onShowTerminal: () => void }) {
  const blocked = useApprovalStore((s) => s.queue.length);
  const workingUntil = useAgentStatusStore((s) => s.workingUntil);
  const workingSince = useAgentStatusStore((s) => s.workingSince);
  const waiting = useAgentStatusStore((s) => s.waiting);
  const [, force] = useState(0);

  // Tick every second while working: keeps the elapsed readout live and also
  // catches the decay-window fallback when no Stop hook fires.
  useEffect(() => {
    if (workingUntil - Date.now() <= 0) return;
    const t = setInterval(() => force((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [workingUntil]);

  const state: "blocked" | "waiting" | "working" | "idle" =
    blocked > 0 ? "blocked" : waiting ? "waiting" : Date.now() < workingUntil ? "working" : "idle";

  if (state === "idle") return null;

  if (state === "waiting" && waiting) {
    // The CLI said so itself (Notification hook): it sits at its own prompt.
    // Not a queued approval — nothing to click here but the terminal.
    const what =
      waiting.kind === "permission_prompt"
        ? "a permission prompt in the terminal"
        : waiting.kind === "agent_needs_input"
          ? "your input in the terminal"
          : "an MCP dialog in the terminal";
    return (
      <button
        className="sb-item sb-clickable sb-agent sb-agent-blocked"
        onClick={onShowTerminal}
        title={`Claude is waiting on ${what} (reported by its Notification hook). Answer it there.`}
      >
        <span className="sb-agent-dot" />
        <span>waiting on you · terminal</span>
      </button>
    );
  }

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

  const elapsed = workingSince > 0 ? formatWorking(Date.now() - workingSince) : null;
  return (
    <span
      className="sb-item sb-agent sb-agent-working"
      title="Working since the last prompt. Ends on the agent's turn-completed signal (Stop hook); falls back to an activity window where the hook isn't installed."
    >
      <span className="sb-agent-dot" />
      <span>working…{elapsed ? ` ${elapsed}` : ""}</span>
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
    if (
      session.status === "live" &&
      profile.supportsLiveModelSwitch &&
      profile.liveModelSwitchInput
    ) {
      const detail: TermInputDetail = {
        sessionId: session.id,
        data: profile.liveModelSwitchInput(model),
      };
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
          <div className="model-menu-head">
            {canLiveSwitch ? "Switch model" : "Model on resume"}
          </div>
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
              Sends <code>/model</code> to the terminal — only takes effect if {profile.label} is
              idle.
            </div>
          )}
          {live && !profile.supportsLiveModelSwitch && (
            <div className="model-menu-note">Applies the next time this session is resumed.</div>
          )}
        </div>
      )}
    </div>
  );
}

/// What the active session's agent was last fed from workspace memory (E1).
/// Renders only after an injection actually happened for THIS session — the
/// transparency half of memory injection: nothing reaches the agent silently.
function MemoryPill({ termId }: { termId: string }) {
  const record = useInjectStore((s) => s.lastByTerm[termId]);
  const feedback = useInjectStore((s) => s.feedback);
  const vote = useInjectStore((s) => s.vote);
  const project = useSessionStore((s) => s.project);
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

  if (!record || record.hits.length === 0) return null;

  return (
    <div className="model-pill-wrap" ref={wrapRef}>
      <button
        className="sb-item sb-clickable"
        onClick={() => setOpen((v) => !v)}
        title="Workspace memories injected into this session's last prompt — click to see what the agent was fed"
      >
        <span className="model-pill-glyph">◈</span>
        <span>
          {record.hits.length} memor{record.hits.length === 1 ? "y" : "ies"}
        </span>
      </button>
      {open && (
        <div className="model-menu" role="menu">
          <div className="model-menu-head">Injected with “{record.promptHead}”</div>
          {record.hits.map((h, i) => {
            const fb = feedback[h.id];
            return (
              <div key={i} className="model-menu-item inject-hit">
                <span className="model-menu-icon">
                  {h.kind === "skill" ? "▤" : h.kind === "profile" ? "⬢" : "◆"}
                </span>
                <span className="model-menu-intent">{h.title}</span>
                <span className="model-menu-model">{Math.round(h.score * 100)}%</span>
                {h.kind !== "profile" && (
                  <span className="inject-vote">
                    <button
                      className="inject-vote-btn"
                      title={`This was useful${fb?.helpful ? ` (${fb.helpful})` : ""} — useful docs rank higher for injection`}
                      onClick={() => project && void vote(project.root, h.id, true)}
                    >
                      👍{fb?.helpful ? ` ${fb.helpful}` : ""}
                    </button>
                    <button
                      className="inject-vote-btn"
                      title={`This got in the way${fb?.unhelpful ? ` (${fb.unhelpful})` : ""} — 3× without a 👍 stops it from being injected`}
                      onClick={() => project && void vote(project.root, h.id, false)}
                    >
                      👎{fb?.unhelpful ? ` ${fb.unhelpful}` : ""}
                    </button>
                  </span>
                )}
              </div>
            );
          })}
          <div className="model-menu-note">
            Retrieved from this project&apos;s memory by semantic match. Your votes reweight future
            injections. Toggle in Context → Memory injection.
          </div>
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

  const pct = progress?.total ? Math.round((progress.downloaded / progress.total) * 100) : null;
  const label =
    phase === "off"
      ? "voice off"
      : phase === "loading"
        ? pct != null
          ? `voice ${pct}%`
          : "voice loading…"
        : phase === "listening"
          ? "listening…"
          : phase === "transcribing"
            ? "transcribing…"
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

/// Context-usage indicator for the active agent session. Reads the on-disk
/// transcript via `session_usage` (Claude's `~/.claude/projects` jsonl or
/// Codex's `~/.codex/sessions` rollout) and shows how full the model context is
/// (`contextTokens / contextWindow`). Polls while the session is live so it
/// tracks the agent's progress; the totals live in the tooltip. Turns amber
/// past 80% as a hint to compact.
function UsagePill({
  termId,
  sessionId,
  projectRoot,
  agent,
  live,
}: {
  termId: string;
  sessionId: string;
  projectRoot: string;
  agent?: string;
  live: boolean;
}) {
  const [usage, setUsage] = useState<SessionUsage | null>(null);
  // The CLI's own numbers, per status-line render (T3). While fresh they are
  // the truth and the transcript poll stands down; when they go stale (idle
  // session, Codex, older CLI) the poll is the fallback it always was.
  const liveStatus = useLiveStatusStore((s) => freshStatus(s.byTerm, termId, Date.now()));
  const hasLive = !!liveStatus && (liveStatus.contextUsed ?? 0) > 0;

  useEffect(() => {
    if (hasLive) return;
    let cancelled = false;
    const load = () => {
      ipc
        .sessionUsage(sessionId, projectRoot, agent)
        .then((u) => {
          if (!cancelled) setUsage(u);
        })
        .catch(() => {
          /* transcript not ready / unreadable — keep last value */
        });
    };
    load();
    // Only poll while the agent can still be producing tokens.
    const t = live ? window.setInterval(load, 5000) : null;
    return () => {
      cancelled = true;
      if (t) window.clearInterval(t);
    };
  }, [sessionId, projectRoot, agent, live, hasLive]);

  if (hasLive && liveStatus) {
    const used = liveStatus.contextUsed ?? 0;
    const size = liveStatus.contextSize ?? 0;
    const pct =
      liveStatus.usedPct !== undefined
        ? Math.round(liveStatus.usedPct)
        : size > 0
          ? Math.round((used / size) * 100)
          : 0;
    const warn = pct >= 80;
    const cost = liveStatus.costUsd !== undefined ? `$${liveStatus.costUsd.toFixed(2)}` : null;
    const tip =
      `Context window: ${fmtTokens(used)} / ${size > 0 ? fmtTokens(size) : "?"} (${pct}%) — reported by the CLI\n` +
      (liveStatus.modelName
        ? `Model: ${liveStatus.modelName} (${liveStatus.modelId ?? ""})\n`
        : "") +
      (cost ? `Session cost: ${cost} (estimated at list price)\n` : "") +
      (liveStatus.linesAdded !== undefined
        ? `Lines: +${liveStatus.linesAdded} / -${liveStatus.linesRemoved ?? 0}\n`
        : "") +
      (liveStatus.inputTotal !== undefined
        ? `Input (cumulative): ${fmtTokens(liveStatus.inputTotal)}\n`
        : "") +
      (liveStatus.outputTotal !== undefined
        ? `Output (cumulative): ${fmtTokens(liveStatus.outputTotal)}`
        : "");
    return (
      <span className={`sb-item sb-muted usage-pill ${warn ? "usage-warn" : ""}`} title={tip}>
        <span className="usage-glyph">⌁</span>
        <span>
          {fmtTokens(used)} ({pct}%){cost ? ` · ${cost}` : ""}
        </span>
      </span>
    );
  }

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
      <span>
        {fmtTokens(usage.contextTokens)} ({pct}%)
      </span>
    </span>
  );
}

/// Compact token count: 1234 → "1.2k", 84000 → "84k", 512 → "512".
function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  const k = n / 1000;
  return `${k >= 10 ? Math.round(k) : k.toFixed(1)}k`;
}
