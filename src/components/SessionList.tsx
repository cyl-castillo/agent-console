import { useEffect, useState } from "react";

import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useSessionStore } from "../stores/sessionStore";
import { useUIStore } from "../stores/uiStore";

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

  const onNew = () => {
    if (!project) return;
    add(project.root);
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
          className="sessions-new"
          onClick={onNew}
          disabled={!project}
          title="New terminal session"
        >+ new</button>
      </div>
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
