import { useEffect, useState } from "react";

import { useRoundtableStore } from "../stores/roundtableStore";
import { useSessionStore } from "../stores/sessionStore";
import type { RoomSummary } from "../types/domain";

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}

function relTime(ms: number, now: number): string {
  const d = Math.max(0, now - ms);
  if (d < 60_000) return "just now";
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return `${Math.floor(d / 86_400_000)}d ago`;
}

/// Saved rooms for the open project. Clicking one re-hydrates the room panel
/// read-only (it never resumes the agents). `onOpenRoom` switches the workbench
/// to the room tab so the re-hydrated conversation is visible.
export function RoomsList({ onOpenRoom }: { onOpenRoom: (id: string) => void }) {
  const rooms = useRoundtableStore((s) => s.rooms);
  const loadRooms = useRoundtableStore((s) => s.loadRooms);
  const deleteSavedRoom = useRoundtableStore((s) => s.deleteSavedRoom);
  const activeId = useRoundtableStore((s) => s.runId);
  const readOnly = useRoundtableStore((s) => s.readOnly);
  const projectRoot = useSessionStore((s) => s.project?.root);
  // A live room autosaves on every turn, so reload when the project changes or
  // the active conversation advances — the list stays fresh without an event.
  const phase = useRoundtableStore((s) => s.phase);
  const turn = useRoundtableStore((s) => s.turn);

  useEffect(() => {
    void loadRooms();
  }, [loadRooms, projectRoot, phase, turn]);

  if (rooms.length === 0) {
    return <div className="sessions-empty">No saved rooms yet.</div>;
  }

  return (
    <ul className="sessions-list">
      {rooms.map((r) => (
        <RoomRow
          key={r.id}
          room={r}
          active={readOnly && r.id === activeId}
          onOpen={() => onOpenRoom(r.id)}
          onDelete={() => {
            const label = r.problem.trim().slice(0, 60) || "this room";
            if (confirm(`Delete saved room "${label}"? This can't be undone.`)) {
              void deleteSavedRoom(r.id);
            }
          }}
        />
      ))}
    </ul>
  );
}

function RoomRow({
  room,
  active,
  onOpen,
  onDelete,
}: {
  room: RoomSummary;
  active: boolean;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const now = useNow(30_000);
  const meta = `${room.lastTurn}t · ${relTime(room.updatedAtMs, now)}`;
  const roster = room.participantNames.join(" · ");

  return (
    <li
      className={`session-row ${active ? "active" : ""}`}
      onClick={onOpen}
      title={`${room.problem || "(untitled room)"}\n${roster} · ${room.messageCount} messages`}
    >
      <span className="session-dot stopped" />
      <span className="session-name">{room.problem || "(untitled room)"}</span>
      <span className="session-meta">{meta}</span>
      <button
        className="session-close"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
        title="Delete saved room"
      >
        ×
      </button>
    </li>
  );
}
