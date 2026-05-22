import { useState } from "react";

import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useSessionStore } from "../stores/sessionStore";
import { useUIStore } from "../stores/uiStore";

export function SessionList() {
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const add = useTerminalsStore((s) => s.add);
  const resume = useTerminalsStore((s) => s.resume);
  const setActive = useTerminalsStore((s) => s.setActive);
  const close = useTerminalsStore((s) => s.close);
  const rename = useTerminalsStore((s) => s.rename);
  const persist = useTerminalsStore((s) => s.persist);
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

  return (
    <div className="sessions">
      <div className="sessions-header">
        <span>Sessions</span>
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
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function SessionRow({ session, active, onActivate, onClose, onRename }: {
  session: TerminalSession;
  active: boolean;
  onActivate: () => void;
  onClose: () => void;
  onRename: (name: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(session.name);

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
      <button
        className="session-close"
        onClick={(e) => { e.stopPropagation(); onClose(); }}
        title="Close session"
      >×</button>
    </li>
  );
}
