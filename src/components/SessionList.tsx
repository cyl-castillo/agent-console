import { useEffect, useState } from "react";

import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useApprovalStore, blockedSessionIds } from "../stores/approvalStore";
import { useSessionStore } from "../stores/sessionStore";
import { useUIStore } from "../stores/uiStore";
import { useToastStore } from "../stores/toastStore";
import { useModelStore, isValidModel, modelLabel } from "../stores/modelStore";
import { AGENT_PROFILES, profileFor, DEFAULT_AGENT, type AgentKind } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import type { BranchInfo } from "../types/domain";

/// The worktree opt-in from the chooser: branch-name component + base branch.
export interface WorktreePick {
  name: string;
  base: string;
}

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}

function formatUptime(ms: number): string {
  if (ms < 60_000) return `${Math.max(1, Math.floor(ms / 1000))}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m`;
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h`;
  return `${Math.floor(ms / 86_400_000)}d`;
}

export function SessionList() {
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const add = useTerminalsStore((s) => s.add);
  const resume = useTerminalsStore((s) => s.resume);
  const setActive = useTerminalsStore((s) => s.setActive);
  const close = useTerminalsStore((s) => s.close);
  const rename = useTerminalsStore((s) => s.rename);
  const persist = useTerminalsStore((s) => s.persist);
  const archive = useTerminalsStore((s) => s.archive);
  const project = useSessionStore((s) => s.project);
  const setTab = useUIStore((s) => s.setTab);
  const setDefaultFor = useModelStore((s) => s.setDefaultFor);
  const defaultAgentFor = useModelStore((s) => s.defaultAgentFor);
  const setDefaultAgentFor = useModelStore((s) => s.setDefaultAgentFor);
  const [choosing, setChoosing] = useState(false);
  const approvalQueue = useApprovalStore((s) => s.queue);
  const blocked = blockedSessionIds(approvalQueue, sessions);

  const lastAgent = project ? defaultAgentFor(project.root) : undefined;

  // Always go through the chooser so the agent and model are explicit, visible
  // choices (no silent fall-back to a default). `undefined` model = account
  // default; agent defaults to Claude. With a worktree pick, the session runs
  // in its own checkout on an `agent/<name>` branch instead of the project root.
  const createSession = async (agent: AgentKind, model?: string, wt?: WorktreePick) => {
    if (!project) return;
    const m = isValidModel(model) ? model : undefined;
    if (wt) {
      try {
        const created = await ipc.worktreeCreate(wt.name, wt.base);
        add(created.info.path, wt.name, m, agent, created.info, created.setupCommand ?? undefined);
        const copies = created.copied.length ? ` · copied ${created.copied.join(", ")}` : "";
        useToastStore
          .getState()
          .show(`Worktree ready on ${created.info.branch}${copies}`, "success");
      } catch (e) {
        // Keep the chooser open so the user can fix the name/base and retry.
        useToastStore.getState().show(`Couldn't create worktree: ${e}`, "error");
        return;
      }
    } else {
      add(project.root, undefined, m, agent);
    }
    setDefaultFor(project.root, agent, m);
    setDefaultAgentFor(project.root, agent);
    setChoosing(false);
    setTab("terminal");
    persist();
  };

  const onActivate = (s: TerminalSession) => {
    if (s.status === "stopped") {
      resume(s.id);
    } else {
      setActive(s.id);
    }
    setTab("terminal");
  };

  const liveCount = sessions.filter((s) => s.status === "live").length;
  const visible = sessions.filter((s) => !s.archived);
  const archived = sessions.filter((s) => s.archived);
  const [historyOpen, setHistoryOpen] = useState(false);

  return (
    <div className="sessions">
      <div className="sessions-actions">
        {liveCount > 0 && <span className="sessions-count">{liveCount} live</span>}
        <span className="spacer" />
        <button
          className={`sessions-new ${choosing ? "open" : ""}`}
          onClick={() => setChoosing((v) => !v)}
          disabled={!project}
          title="New session — pick an agent and model"
        >
          + new
        </button>
      </div>
      {choosing && project && (
        <AgentModelChooser
          projectRoot={project.root}
          lastAgent={lastAgent ?? DEFAULT_AGENT}
          onPick={createSession}
          onCancel={() => setChoosing(false)}
        />
      )}
      {visible.length === 0 && archived.length === 0 ? (
        <div className="sessions-empty">No sessions yet. Click + new.</div>
      ) : (
        <ul className="sessions-list">
          {visible.map((s) => (
            <SessionRow
              key={s.id}
              session={s}
              active={s.id === activeId}
              blocked={blocked.has(s.id)}
              onActivate={() => onActivate(s)}
              onClose={async () => {
                if (
                  s.status === "live" &&
                  !confirm(`Close session "${s.name}"? Process will be killed.`)
                )
                  return;
                await close(s.id);
              }}
              onRename={(name) => {
                rename(s.id, name);
                persist();
              }}
              onArchive={s.status === "stopped" ? () => archive(s.id) : undefined}
            />
          ))}
        </ul>
      )}

      {archived.length > 0 && (
        <div className="sessions-history">
          <button
            className={`sessions-history-toggle ${historyOpen ? "open" : ""}`}
            onClick={() => setHistoryOpen((v) => !v)}
            title="Archived sessions — resumable anytime, never auto-deleted"
          >
            {historyOpen ? "▾" : "▸"} History
            <span className="sessions-history-count">{archived.length}</span>
          </button>
          {historyOpen && (
            <ul className="sessions-list sessions-history-list">
              {archived.map((s) => (
                <li key={s.id} className="session-row stopped history">
                  <span className="session-name" title={s.name}>
                    {s.name}
                  </span>
                  <span className="session-meta">
                    {formatUptime(Math.max(0, Date.now() - (s.lastActiveMs ?? s.createdAtMs)))} ago
                  </span>
                  <button
                    className="session-resume"
                    onClick={() => onActivate(s)}
                    title="Resume this session (moves it back to the list)"
                  >
                    ▸
                  </button>
                  <button
                    className="session-close"
                    onClick={async () => {
                      if (confirm(`Delete archived session "${s.name}"? This is permanent.`))
                        await close(s.id);
                    }}
                    title="Delete permanently"
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

/// Inline, in-flow agent + model chooser shown under the "+ new" button.
/// Rendered in the normal layout (not an absolutely-positioned popover) so it
/// never gets clipped by the 240px sidebar's `overflow: hidden`. First pick the
/// agent, then a model/tuning preset for that agent; picking creates the session
/// right away. There is no silent default — both choices are explicit. The model
/// per-project default is remembered separately for each agent.
function AgentModelChooser({
  projectRoot,
  lastAgent,
  onPick,
  onCancel,
}: {
  projectRoot: string;
  lastAgent: AgentKind;
  onPick: (agent: AgentKind, model?: string, wt?: WorktreePick) => void;
  onCancel: () => void;
}) {
  const defaultFor = useModelStore((s) => s.defaultFor);
  const [agent, setAgent] = useState<AgentKind>(lastAgent);
  const [showCustom, setShowCustom] = useState(false);
  const [custom, setCustom] = useState("");
  const [wtOn, setWtOn] = useState(false);
  const [wtName, setWtName] = useState("");
  const [wtBase, setWtBase] = useState("");
  const [branches, setBranches] = useState<BranchInfo[] | null>(null);

  const profile = profileFor(agent);
  const lastModel = defaultFor(projectRoot, agent);

  // Load branches on first worktree opt-in; default the base to the current one.
  useEffect(() => {
    if (!wtOn || branches !== null) return;
    ipc
      .gitBranches()
      .then((bs) => {
        setBranches(bs);
        const current = bs.find((b) => b.current);
        if (current && !wtBase) setWtBase(current.name);
      })
      .catch(() => setBranches([]));
  }, [wtOn, branches, wtBase]);

  const worktreePick = (): WorktreePick | undefined => {
    if (!wtOn) return undefined;
    const name = wtName.trim() || `session-${Date.now().toString(36)}`;
    return { name, base: wtBase };
  };

  const commitCustom = () => {
    const v = custom.trim();
    if (isValidModel(v)) onPick(agent, v, worktreePick());
  };

  return (
    <div className="model-chooser" role="listbox" aria-label="Choose an agent and model">
      {AGENT_PROFILES.length > 1 && (
        <div className="agent-chooser-tabs" role="tablist" aria-label="Agent">
          {AGENT_PROFILES.map((p) => (
            <button
              key={p.kind}
              role="tab"
              aria-selected={agent === p.kind}
              className={`agent-chooser-tab ${agent === p.kind ? "active" : ""}`}
              onClick={() => {
                setAgent(p.kind);
                setShowCustom(false);
              }}
            >
              <span className="agent-chooser-tab-icon">{p.icon}</span>
              <span>{p.label}</span>
            </button>
          ))}
        </div>
      )}

      {profile.models.map((p) => (
        <button
          key={p.value}
          className={`model-chooser-item ${lastModel === p.value ? "last" : ""}`}
          onClick={() => onPick(agent, p.value, worktreePick())}
          autoFocus={lastModel === p.value}
        >
          <span className="model-chooser-icon">{p.icon}</span>
          <span className="model-chooser-text">
            <span className="model-chooser-intent">{p.intent}</span>
            <span className="model-chooser-model">{p.label}</span>
          </span>
          {lastModel === p.value && (
            <span className="model-chooser-last-dot" title="last used">
              ●
            </span>
          )}
        </button>
      ))}

      <div className="wt-opt">
        <label className="wt-opt-toggle">
          <input type="checkbox" checked={wtOn} onChange={(e) => setWtOn(e.target.checked)} />
          <span>Isolated worktree</span>
        </label>
        {wtOn && (
          <div className="wt-opt-fields">
            <input
              className="wb-search-input"
              placeholder="branch name (agent/…)"
              value={wtName}
              onChange={(e) => setWtName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") onCancel();
              }}
            />
            <select
              className="wt-opt-base"
              value={wtBase}
              onChange={(e) => setWtBase(e.target.value)}
              title="Base branch — the session branches off this and merges back into it"
            >
              {branches === null && <option value="">loading branches…</option>}
              {branches?.map((b) => (
                <option key={b.name} value={b.name}>
                  {b.current ? `${b.name} (current)` : b.name}
                </option>
              ))}
            </select>
            <span className="wt-opt-hint">
              Own checkout + branch — your files stay untouched; merge or discard when done.
            </span>
          </div>
        )}
      </div>

      <div className="model-chooser-foot">
        <button
          className="model-chooser-link btn btn-ghost"
          onClick={() => onPick(agent, undefined, worktreePick())}
        >
          {agent === "codex" ? "Config default" : "Account default"}
        </button>
        <span className="model-chooser-sep">·</span>
        <button
          className="model-chooser-link btn btn-ghost"
          onClick={() => setShowCustom((v) => !v)}
        >
          Custom…
        </button>
        <span className="spacer" />
        <button className="model-chooser-link btn btn-ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>

      {showCustom && (
        <div className="model-chooser-custom">
          <input
            className="wb-search-input"
            autoFocus
            placeholder={
              agent === "codex"
                ? "reasoning effort (e.g. xhigh)"
                : "model id (e.g. claude-opus-4-8)"
            }
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitCustom();
              if (e.key === "Escape") onCancel();
            }}
          />
        </div>
      )}
    </div>
  );
}

function SessionRow({
  session,
  active,
  blocked,
  onActivate,
  onClose,
  onRename,
  onArchive,
}: {
  session: TerminalSession;
  active: boolean;
  /// A queued approval is attributed to this session — it's waiting on you.
  blocked: boolean;
  onActivate: () => void;
  onClose: () => void;
  onRename: (name: string) => void;
  /// Present only for stopped sessions (live ones must be closed first).
  onArchive?: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(session.name);
  const now = useNow(30_000);
  const meta =
    session.status === "live" ? formatUptime(Math.max(0, now - session.createdAtMs)) : "stopped";

  const commit = () => {
    const v = draft.trim();
    if (v && v !== session.name) onRename(v);
    setEditing(false);
  };

  return (
    <li
      className={`session-row ${active ? "active" : ""} ${session.status === "stopped" ? "stopped" : ""}`}
      onClick={onActivate}
      title={session.cwd}
    >
      <span className={`session-dot ${session.status}`} />
      {editing ? (
        <input
          className="session-name-input"
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") {
              setDraft(session.name);
              setEditing(false);
            }
          }}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span
          className="session-name"
          onDoubleClick={(e) => {
            e.stopPropagation();
            setEditing(true);
          }}
        >
          {session.name}
        </span>
      )}
      <span className="session-agent" title={`Agent: ${profileFor(session.agent).label}`}>
        {profileFor(session.agent).icon}
      </span>
      {session.worktree && (
        <span
          className="session-branch"
          title={`Isolated worktree · ${session.worktree.branch} → ${session.worktree.baseBranch}\n${session.worktree.path}`}
        >
          ⎇
        </span>
      )}
      {session.model && (
        <span
          className="session-model"
          title={`${profileFor(session.agent).label} · ${modelLabel(session.model, session.agent)}`}
        >
          {modelLabel(session.model, session.agent)}
        </span>
      )}
      {blocked && (
        <span className="session-blocked" title="Waiting for you to approve an action">
          waiting
        </span>
      )}
      <span className="session-meta">{meta}</span>
      {onArchive && (
        <button
          className="session-archive"
          onClick={(e) => {
            e.stopPropagation();
            onArchive();
          }}
          title="Archive — hide in History, resumable anytime"
        >
          ↓
        </button>
      )}
      <button
        className="session-close"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        title="Close session"
      >
        ×
      </button>
    </li>
  );
}
