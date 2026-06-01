import { useEffect, useState } from "react";

import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useSessionStore } from "../stores/sessionStore";
import { useUIStore } from "../stores/uiStore";
import { useModelStore, isValidModel, modelLabel } from "../stores/modelStore";
import { AGENT_PROFILES, profileFor, DEFAULT_AGENT, type AgentKind } from "../agents/profiles";

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
  const acceptSuggestion = useTerminalsStore((s) => s.acceptSuggestion);
  const dismissSuggestion = useTerminalsStore((s) => s.dismissSuggestion);
  const project = useSessionStore((s) => s.project);
  const setTab = useUIStore((s) => s.setTab);
  const setDefaultFor = useModelStore((s) => s.setDefaultFor);
  const defaultAgentFor = useModelStore((s) => s.defaultAgentFor);
  const setDefaultAgentFor = useModelStore((s) => s.setDefaultAgentFor);
  const [choosing, setChoosing] = useState(false);

  const lastAgent = project ? defaultAgentFor(project.root) : undefined;

  // Always go through the chooser so the agent and model are explicit, visible
  // choices (no silent fall-back to a default). `undefined` model = account
  // default; agent defaults to Claude.
  const createSession = (agent: AgentKind, model?: string) => {
    if (!project) return;
    const m = isValidModel(model) ? model : undefined;
    add(project.root, undefined, m, agent);
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
        >+ new</button>
      </div>
      {choosing && project && (
        <AgentModelChooser
          projectRoot={project.root}
          lastAgent={lastAgent ?? DEFAULT_AGENT}
          onPick={createSession}
          onCancel={() => setChoosing(false)}
        />
      )}
      {sessions.length === 0 ? (
        <div className="sessions-empty">No sessions yet. Click + new.</div>
      ) : (
        <ul className="sessions-list">
          {sessions.map((s) => (
            <SessionRow
              key={s.id}
              session={s}
              active={s.id === activeId}
              onActivate={() => onActivate(s)}
              onClose={async () => {
                if (s.status === "live" && !confirm(`Close session "${s.name}"? Process will be killed.`)) return;
                await close(s.id);
              }}
              onRename={(name) => { rename(s.id, name); persist(); }}
              onAcceptSuggestion={() => { acceptSuggestion(s.id); }}
              onDismissSuggestion={() => { dismissSuggestion(s.id); }}
            />
          ))}
        </ul>
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
function AgentModelChooser({ projectRoot, lastAgent, onPick, onCancel }: {
  projectRoot: string;
  lastAgent: AgentKind;
  onPick: (agent: AgentKind, model?: string) => void;
  onCancel: () => void;
}) {
  const defaultFor = useModelStore((s) => s.defaultFor);
  const [agent, setAgent] = useState<AgentKind>(lastAgent);
  const [showCustom, setShowCustom] = useState(false);
  const [custom, setCustom] = useState("");

  const profile = profileFor(agent);
  const lastModel = defaultFor(projectRoot, agent);

  const commitCustom = () => {
    const v = custom.trim();
    if (isValidModel(v)) onPick(agent, v);
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
              onClick={() => { setAgent(p.kind); setShowCustom(false); }}
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
          onClick={() => onPick(agent, p.value)}
          autoFocus={lastModel === p.value}
        >
          <span className="model-chooser-icon">{p.icon}</span>
          <span className="model-chooser-text">
            <span className="model-chooser-intent">{p.intent}</span>
            <span className="model-chooser-model">{p.label}</span>
          </span>
          {lastModel === p.value && <span className="model-chooser-last-dot" title="last used">●</span>}
        </button>
      ))}

      <div className="model-chooser-foot">
        <button className="model-chooser-link" onClick={() => onPick(agent, undefined)}>
          {agent === "codex" ? "Config default" : "Account default"}
        </button>
        <span className="model-chooser-sep">·</span>
        <button className="model-chooser-link" onClick={() => setShowCustom((v) => !v)}>
          Custom…
        </button>
        <span className="spacer" />
        <button className="model-chooser-link" onClick={onCancel}>Cancel</button>
      </div>

      {showCustom && (
        <div className="model-chooser-custom">
          <input
            className="wb-search-input"
            autoFocus
            placeholder={agent === "codex" ? "reasoning effort (e.g. xhigh)" : "model id (e.g. claude-opus-4-8)"}
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

function SessionRow({ session, active, onActivate, onClose, onRename, onAcceptSuggestion, onDismissSuggestion }: {
  session: TerminalSession;
  active: boolean;
  onActivate: () => void;
  onClose: () => void;
  onRename: (name: string) => void;
  onAcceptSuggestion: () => void;
  onDismissSuggestion: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(session.name);
  const now = useNow(30_000);
  const meta = session.status === "live"
    ? formatUptime(Math.max(0, now - session.createdAtMs))
    : "stopped";

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
            if (e.key === "Escape") { setDraft(session.name); setEditing(false); }
          }}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span
          className="session-name"
          onDoubleClick={(e) => { e.stopPropagation(); setEditing(true); }}
        >{session.name}</span>
      )}
      <span className="session-agent" title={`Agent: ${profileFor(session.agent).label}`}>
        {profileFor(session.agent).icon}
      </span>
      {session.model && (
        <span className="session-model" title={`${profileFor(session.agent).label} · ${modelLabel(session.model, session.agent)}`}>
          {modelLabel(session.model, session.agent)}
        </span>
      )}
      <span className="session-meta">{meta}</span>
      <button
        className="session-close"
        onClick={(e) => { e.stopPropagation(); onClose(); }}
        title="Close session"
      >×</button>
      {session.suggestedName && session.suggestedName !== session.name && !editing && (
        <div className="session-suggestion" onClick={(e) => e.stopPropagation()}>
          <span className="session-suggestion-label">
            Rename to <strong>“{session.suggestedName}”</strong>?
          </span>
          <button
            className="session-suggestion-accept"
            onClick={onAcceptSuggestion}
            title="Accept suggestion"
          >✓</button>
          <button
            className="session-suggestion-dismiss"
            onClick={onDismissSuggestion}
            title="Dismiss"
          >✕</button>
        </div>
      )}
    </li>
  );
}
