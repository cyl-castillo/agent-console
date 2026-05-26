import { useEffect, useState } from "react";

import { SessionList } from "./SessionList";
import { ChangesList } from "./ChangesList";
import { FileTree } from "./FileTree";
import { useSessionStore } from "../stores/sessionStore";
import { useChangesStore } from "../stores/changesStore";

type SectionId = "sessions" | "changes" | "files";

export function LeftSidebar() {
  const tree = useSessionStore((s) => s.tree);
  const changesCount = useChangesStore((s) => s.status?.changes.length ?? 0);

  // Files starts collapsed when there are changes (focus on what the agent did);
  // expands automatically when the working tree is clean.
  const [collapsed, setCollapsed] = useState<Record<SectionId, boolean>>({
    sessions: false,
    changes: false,
    files: true,
  });

  useEffect(() => {
    setCollapsed((prev) => ({ ...prev, files: changesCount > 0 }));
  }, [changesCount]);

  const toggle = (id: SectionId) =>
    setCollapsed((prev) => ({ ...prev, [id]: !prev[id] }));

  return (
    <aside className="panel left">
      <Section
        id="sessions"
        title="Sessions"
        collapsed={collapsed.sessions}
        onToggle={toggle}
      >
        <SessionList />
      </Section>

      <Section
        id="changes"
        title="Changes"
        badge={changesCount > 0 ? changesCount : undefined}
        collapsed={collapsed.changes}
        onToggle={toggle}
      >
        <ChangesList />
      </Section>

      <Section
        id="files"
        title="Files"
        collapsed={collapsed.files}
        onToggle={toggle}
        grow
      >
        {tree ? <FileTree root={tree} /> : <div className="placeholder">Loading…</div>}
      </Section>
    </aside>
  );
}

function Section({
  id,
  title,
  badge,
  collapsed,
  onToggle,
  grow,
  children,
}: {
  id: SectionId;
  title: string;
  badge?: number;
  collapsed: boolean;
  onToggle: (id: SectionId) => void;
  grow?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section className={`side-section ${collapsed ? "collapsed" : ""} ${grow ? "grow" : ""}`}>
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
