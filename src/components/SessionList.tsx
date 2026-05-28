import { useEffect, useState } from "react";

import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useSessionStore } from "../stores/sessionStore";
import { useUIStore } from "../stores/uiStore";
import { useModelStore, MODEL_PRESETS, isValidModel, modelLabel } from "../stores/modelStore";

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
  const defaultFor = useModelStore((s) => s.defaultFor);
  const setDefaultFor = useModelStore((s) => s.setDefaultFor);
  const [choosing, setChoosing] = useState(false);

  const lastModel = project ? defaultFor(project.root) : undefined;

  // Always go through the chooser so the model is an explicit, visible choice
  // (no silent fall-back to a default). `undefined` model = account default.
  const createSession = (model?: string) => {
    if (!project) return;
    const m = isValidModel(model) ? model : undefined;
    add(project.root, undefined, m);
    setDefaultFor(project.root, m);
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
          title="New session — pick a model"
        >+ new</button>
      </div>
      {choosing && (
        <ModelChooser
          lastModel={lastModel}
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

/// Inline, in-flow model chooser shown under the "+ new" button. Rendered in
/// the normal layout (not an absolutely-positioned popover) so it never gets
/// clipped by the 240px sidebar's `overflow: hidden`. Picking creates the
/// session right away; there is no silent default — the choice is explicit.
function ModelChooser({ lastModel, onPick, onCancel }: {
  lastModel?: string;
  onPick: (model?: string) => void;
  onCancel: () => void;
}) {
  const [showCustom, setShowCustom] = useState(false);
  const [custom, setCustom] = useState("");

  const commitCustom = () => {
    const v = custom.trim();
    if (isValidModel(v)) onPick(v);
  };

  return (
    <div className="model-chooser" role="listbox" aria-label="Choose a model">
      {MODEL_PRESETS.map((p) => (
        <button
          key={p.model}
          className={`model-chooser-item ${lastModel === p.model ? "last" : ""}`}
          onClick={() => onPick(p.model)}
          autoFocus={lastModel === p.model}
        >
          <span className="model-chooser-icon">{p.icon}</span>
          <span className="model-chooser-text">
            <span className="model-chooser-intent">{p.intent}</span>
            <span className="model-chooser-model">{p.label}</span>
          </span>
          {lastModel === p.model && <span className="model-chooser-last-dot" title="last used">●</span>}
        </button>
      ))}

      <div className="model-chooser-foot">
        <button className="model-chooser-link" onClick={() => onPick(undefined)}>
          Account default
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
            placeholder="model id (e.g. claude-opus-4-8)"
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
      {session.model && (
        <span className="session-model" title={`Model: ${modelLabel(session.model)}`}>
          {modelLabel(session.model)}
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
