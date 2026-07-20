import { useEffect, useState } from "react";

import { SessionList } from "./SessionList";
import { RoomsList } from "./RoomsList";
import { ChangesList } from "./ChangesList";
import { FileTree } from "./FileTree";
import { useSessionStore } from "../stores/sessionStore";
import { useChangesStore } from "../stores/changesStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useRoundtableStore } from "../stores/roundtableStore";

type SectionId = "sessions" | "rooms" | "changes" | "files";

type CollapsedMap = Record<SectionId, boolean>;
const DEFAULT_COLLAPSED: CollapsedMap = {
  sessions: false,
  rooms: false,
  changes: false,
  files: true,
};

const KEY_PREFIX = "agent-console:sidebar-collapsed:";

function loadCollapsed(projectRoot: string | undefined): CollapsedMap {
  if (!projectRoot) return DEFAULT_COLLAPSED;
  try {
    const raw = localStorage.getItem(KEY_PREFIX + projectRoot);
    if (!raw) return DEFAULT_COLLAPSED;
    const parsed = JSON.parse(raw);
    return { ...DEFAULT_COLLAPSED, ...parsed };
  } catch {
    return DEFAULT_COLLAPSED;
  }
}

function saveCollapsed(projectRoot: string | undefined, v: CollapsedMap) {
  if (!projectRoot) return;
  try {
    localStorage.setItem(KEY_PREFIX + projectRoot, JSON.stringify(v));
  } catch {
    /* ignore */
  }
}

export function LeftSidebar({ onOpenRoom }: { onOpenRoom: (id: string) => void }) {
  const tree = useSessionStore((s) => s.tree);
  const projectRoot = useSessionStore((s) => s.project?.root);
  const status = useChangesStore((s) => s.status);
  const changesCount = status?.changes.length ?? 0;
  const isRepo = status?.isRepo ?? false;
  const sessionsCount = useTerminalsStore((s) => s.sessions.length);
  const roomsCount = useRoundtableStore((s) => s.rooms.length);

  const [collapsed, setCollapsed] = useState<CollapsedMap>(() => loadCollapsed(projectRoot));

  // Re-load persisted state when switching projects.
  useEffect(() => {
    setCollapsed(loadCollapsed(projectRoot));
  }, [projectRoot]);

  // Force-collapse Changes when the project isn't a git repo.
  useEffect(() => {
    if (status && !isRepo) {
      setCollapsed((prev) => (prev.changes ? prev : { ...prev, changes: true }));
    }
  }, [status, isRepo]);

  const toggle = (id: SectionId) =>
    setCollapsed((prev) => {
      const next = { ...prev, [id]: !prev[id] };
      saveCollapsed(projectRoot, next);
      return next;
    });

  return (
    <aside className="panel left">
      <Section
        id="sessions"
        title="Sessions"
        badge={sessionsCount > 0 ? sessionsCount : undefined}
        collapsed={collapsed.sessions}
        onToggle={toggle}
      >
        <SessionList />
      </Section>

      <Section
        id="rooms"
        title="Rooms"
        badge={roomsCount > 0 ? roomsCount : undefined}
        collapsed={collapsed.rooms}
        onToggle={toggle}
      >
        <RoomsList onOpenRoom={onOpenRoom} />
      </Section>

      <Section
        id="changes"
        title="Changes"
        badge={changesCount > 0 ? changesCount : undefined}
        muted={!isRepo}
        collapsed={collapsed.changes}
        onToggle={toggle}
      >
        <ChangesList />
      </Section>

      <Section id="files" title="Files" collapsed={collapsed.files} onToggle={toggle} grow>
        {tree ? <FileTree root={tree} /> : <div className="placeholder">Loading…</div>}
      </Section>
    </aside>
  );
}

function Section({
  id,
  title,
  badge,
  muted,
  collapsed,
  onToggle,
  grow,
  children,
}: {
  id: SectionId;
  title: string;
  badge?: number;
  muted?: boolean;
  collapsed: boolean;
  onToggle: (id: SectionId) => void;
  grow?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section
      className={`side-section ${collapsed ? "collapsed" : ""} ${grow ? "grow" : ""} ${muted ? "muted" : ""}`}
    >
      <button
        className="side-section-header"
        onClick={() => onToggle(id)}
        title={collapsed ? `Expand ${title}` : `Collapse ${title}`}
      >
        <span className="caret">{collapsed ? "▸" : "▾"}</span>
        <span className="title">{title}</span>
        {badge !== undefined && <span className="side-badge">{badge}</span>}
      </button>
      {!collapsed && <div className="side-section-body">{children}</div>}
    </section>
  );
}
