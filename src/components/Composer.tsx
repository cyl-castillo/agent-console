import { useEffect, useRef, useState } from "react";

import { typeIntoActiveSession } from "../lib/termInput";

/// Multi-line prompt drafting box for the terminal pane. The raw PTY is a
/// hostile place to write a long prompt (no wrapping control, no editing
/// comfort); this gives a real textarea. Ctrl+Enter sends the draft to the
/// active agent (typed + submitted — sending is the point here, unlike the
/// review-first note/seed flows where text arrives unrequested). Esc closes,
/// keeping the draft. The draft persists per project across restarts.

function draftKey(projectRoot: string): string {
  return `agent-console:composer-draft:${projectRoot}`;
}

export function Composer({ projectRoot, onClose }: { projectRoot: string; onClose: () => void }) {
  const [draft, setDraft] = useState<string>(() => {
    try { return localStorage.getItem(draftKey(projectRoot)) ?? ""; } catch { return ""; }
  });
  const taRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    taRef.current?.focus();
  }, []);

  // Persist the draft as it changes so an accidental close/crash loses nothing.
  useEffect(() => {
    try { localStorage.setItem(draftKey(projectRoot), draft); } catch { /* ignore */ }
  }, [draft, projectRoot]);

  const send = async () => {
    if (!draft.trim()) return;
    const ok = await typeIntoActiveSession(draft, { submit: true });
    if (ok) {
      setDraft("");
      try { localStorage.removeItem(draftKey(projectRoot)); } catch { /* ignore */ }
      onClose();
    }
  };

  return (
    <div className="composer">
      <textarea
        ref={taRef}
        className="composer-text"
        value={draft}
        placeholder="Draft a prompt… (Enter for new lines — nothing is sent until Ctrl+Enter)"
        spellCheck={false}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void send();
          }
          if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          }
        }}
      />
      <div className="composer-foot">
        <span className="composer-hint">
          Ctrl+Enter sends to the active session · Esc closes (draft kept)
        </span>
        <button
          className="wb-cta wb-cta-sm"
          onClick={() => void send()}
          disabled={!draft.trim()}
        >Send ⏎</button>
      </div>
    </div>
  );
}
