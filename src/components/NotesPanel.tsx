import { useEffect } from "react";

import { useNotesStore, NOTE_COLORS } from "../stores/notesStore";
import { useSessionStore } from "../stores/sessionStore";
import { typeIntoActiveSession } from "../lib/termInput";
import { PanelError } from "./PanelError";
import type { StickyNote } from "../types/domain";

export function NotesPanel() {
  const project = useSessionStore((s) => s.project);
  const notes = useNotesStore((s) => s.notes);
  const loading = useNotesStore((s) => s.loading);
  const error = useNotesStore((s) => s.error);
  const projectRoot = useNotesStore((s) => s.projectRoot);
  const load = useNotesStore((s) => s.load);
  const add = useNotesStore((s) => s.add);

  useEffect(() => {
    if (project && project.root !== projectRoot) void load(project.root);
  }, [project, projectRoot, load]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">notes</span>
        <span className="spacer" />
        <button className="workbench-action" onClick={add} title="New note">＋</button>
      </div>
      <div className="workbench-body">
        {error && <PanelError message={error} onRetry={() => project && void load(project.root)} />}
        {loading && notes.length === 0 ? (
          <div className="wb-hint">Loading…</div>
        ) : notes.length === 0 ? (
          <div className="wb-empty">
            Your scratchpad while agents work: prompt ideas, things to review,
            reminders. Notes stay with this project.
            <button className="wb-cta wb-cta-sm" onClick={add} style={{ marginLeft: 8 }}>
              ＋ First note
            </button>
          </div>
        ) : (
          <div className="notes-grid">
            {notes.map((n) => <NoteCard key={n.id} note={n} />)}
          </div>
        )}
      </div>
    </div>
  );
}

function NoteCard({ note }: { note: StickyNote }) {
  const updateText = useNotesStore((s) => s.updateText);
  const setColor = useNotesStore((s) => s.setColor);
  const remove = useNotesStore((s) => s.remove);

  return (
    <div className={`note-card note-${NOTE_COLORS.includes(note.color as typeof NOTE_COLORS[number]) ? note.color : "yellow"}`}>
      <textarea
        className="note-text"
        value={note.text}
        placeholder="Write…"
        spellCheck={false}
        onChange={(e) => updateText(note.id, e.target.value)}
      />
      <div className="note-foot">
        <div className="note-colors">
          {NOTE_COLORS.map((c) => (
            <button
              key={c}
              className={`note-dot dot-${c} ${c === note.color ? "active" : ""}`}
              onClick={() => setColor(note.id, c)}
              title={c}
              aria-label={`Color ${c}`}
            />
          ))}
        </div>
        <div className="note-actions">
          <button
            className="note-act"
            onClick={() => void sendToAgent(note.text)}
            disabled={!note.text.trim()}
            title="Type this note into the active agent session (you review, then send)"
          >▸</button>
          <button
            className="note-act note-del"
            onClick={() => remove(note.id)}
            title="Delete note"
          >✕</button>
        </div>
      </div>
    </div>
  );
}

/// Note → prompt: type the note into the active session's agent input. Same
/// contract as the Jira seed — never auto-sends; the human reviews and submits.
async function sendToAgent(text: string): Promise<void> {
  await typeIntoActiveSession(text);
}
