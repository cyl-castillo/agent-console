import { useEffect } from "react";

import { useChangesStore } from "../stores/changesStore";
import type { GitFileChange } from "../types/domain";
import { DiffViewer } from "./DiffViewer";

export function ChangesView() {
  const { status, selected, diff, error, refresh, setSelected, revert, revertAll } =
    useChangesStore();

  // First load when mounted (App also triggers on project change).
  useEffect(() => {
    if (!status) refresh();
  }, [status, refresh]);

  if (error) {
    return <div className="placeholder" style={{ color: "var(--danger)" }}>{error}</div>;
  }
  if (!status) {
    return <div className="placeholder">Loading…</div>;
  }
  if (!status.isRepo) {
    return <div className="placeholder">Not a git repository.</div>;
  }

  return (
    <div className="changes">
      <div className="changes-list">
        <div className="changes-list-header">
          <span>
            {status.branch ? `▎ ${status.branch}` : "(detached)"} · {status.changes.length} change(s)
          </span>
          <span style={{ display: "flex", gap: 4 }}>
            <button onClick={refresh}>Refresh</button>
            {status.changes.length > 0 && (
              <button
                onClick={() => {
                  if (confirm(`Revert ALL ${status.changes.length} change(s)? This cannot be undone.`)) {
                    revertAll();
                  }
                }}
                title="Revert all changes"
              >Revert all</button>
            )}
          </span>
        </div>
        {status.changes.length === 0 && (
          <div className="placeholder" style={{ padding: 12 }}>Working tree clean.</div>
        )}
        {status.changes.map((c) => (
          <ChangeRow
            key={c.path}
            change={c}
            active={c.path === selected}
            onClick={() => setSelected(c.path)}
            onRevert={async () => {
              if (confirm(`Revert "${c.path}"? This cannot be undone.`)) {
                await revert(c.path);
              }
            }}
          />
        ))}
      </div>
      <div className="changes-diff">
        {selected ? <DiffViewer diff={diff} /> : <div className="placeholder">Select a file.</div>}
      </div>
    </div>
  );
}

function ChangeRow({ change, active, onClick, onRevert }: {
  change: GitFileChange;
  active: boolean;
  onClick: () => void;
  onRevert: () => void;
}) {
  const badge = badgeFor(change);
  return (
    <div className={`change-row ${active ? "active" : ""}`} onClick={onClick} title={change.code}>
      <span className={`change-badge ${badge.cls}`}>{badge.label}</span>
      <span className="change-path">{change.path}</span>
      <button
        className="change-revert"
        onClick={(e) => { e.stopPropagation(); onRevert(); }}
        title="Revert this file"
      >
        ↺
      </button>
    </div>
  );
}

function badgeFor(c: GitFileChange): { label: string; cls: string } {
  if (c.untracked) return { label: "U", cls: "untracked" };
  const x = c.code[0];
  const y = c.code[1];
  const ch = y !== " " ? y : x;
  switch (ch) {
    case "M": return { label: "M", cls: "modified" };
    case "A": return { label: "A", cls: "added" };
    case "D": return { label: "D", cls: "deleted" };
    case "R": return { label: "R", cls: "modified" };
    default:  return { label: ch ?? "?", cls: "modified" };
  }
}
