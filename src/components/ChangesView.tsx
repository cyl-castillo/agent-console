import { useEffect, useMemo } from "react";

import { useChangesStore } from "../stores/changesStore";
import type { GitFileChange } from "../types/domain";
import { DiffViewer } from "./DiffViewer";

export function ChangesView() {
  const {
    status, selected, diff, error, commitMessage, committing,
    refresh, setSelected, setCommitMessage, stage, unstage, revert, revertAll, commit,
  } = useChangesStore();

  useEffect(() => {
    if (!status) refresh();
  }, [status, refresh]);

  const { staged, unstaged, stagedCount } = useMemo(() => {
    const all = status?.changes ?? [];
    return {
      staged: all.filter((c) => c.staged),
      unstaged: all.filter((c) => !c.staged),
      stagedCount: all.filter((c) => c.staged).length,
    };
  }, [status]);

  if (error) {
    return (
      <div className="changes">
        <div className="placeholder" style={{ color: "var(--danger)", padding: 16 }}>
          {error}
          <div style={{ marginTop: 12 }}>
            <button onClick={refresh}>Retry</button>
          </div>
        </div>
      </div>
    );
  }
  if (!status) {
    return <div className="placeholder">Loading…</div>;
  }
  if (!status.isRepo) {
    return (
      <div className="placeholder" style={{ padding: 24 }}>
        <div style={{ fontSize: 14, marginBottom: 8 }}>Not a git repository</div>
        <div style={{ fontSize: 12 }}>
          Initialize one with <code>git init</code> from the terminal to track changes here.
        </div>
      </div>
    );
  }

  const allChanges = status.changes;
  const canCommit = stagedCount > 0 && commitMessage.trim().length > 0 && !committing;

  const onCommit = async () => {
    const sha = await commit();
    if (sha) {
      // small visible confirmation via title; refresh handled in store
    }
  };

  return (
    <div className="changes">
      <div className="changes-list">
        <div className="changes-list-header">
          <span>
            {status.branch ? `▎ ${status.branch}` : "(detached)"} · {allChanges.length} change{allChanges.length === 1 ? "" : "s"}
          </span>
          <span style={{ display: "flex", gap: 4 }}>
            <button onClick={refresh} title="Refresh status">↻</button>
            {allChanges.length > 0 && (
              <button
                onClick={() => {
                  if (confirm(`Discard ALL ${allChanges.length} change(s)? This cannot be undone.`)) {
                    revertAll();
                  }
                }}
                title="Discard all changes"
              >Discard all</button>
            )}
          </span>
        </div>

        {allChanges.length === 0 ? (
          <div className="placeholder" style={{ padding: 16 }}>
            <div style={{ fontSize: 13 }}>Working tree clean ✓</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>Nothing to commit.</div>
          </div>
        ) : (
          <>
            {staged.length > 0 && (
              <ChangeGroup
                title={`Staged (${staged.length})`}
                changes={staged}
                selected={selected}
                onSelect={setSelected}
                action={{
                  label: "−",
                  title: "Unstage",
                  run: unstage,
                }}
                onRevert={(p) => {
                  if (confirm(`Discard changes in "${p}"? This cannot be undone.`)) revert(p);
                }}
              />
            )}
            {unstaged.length > 0 && (
              <ChangeGroup
                title={`Changes (${unstaged.length})`}
                changes={unstaged}
                selected={selected}
                onSelect={setSelected}
                action={{
                  label: "+",
                  title: "Stage",
                  run: stage,
                }}
                onRevert={(p) => {
                  if (confirm(`Discard changes in "${p}"? This cannot be undone.`)) revert(p);
                }}
              />
            )}
          </>
        )}

        <div className="commit-box">
          <textarea
            className="commit-message"
            placeholder={
              stagedCount === 0
                ? "Stage at least one file to commit…"
                : "Commit message (summary on first line)"
            }
            value={commitMessage}
            onChange={(e) => setCommitMessage(e.target.value)}
            rows={3}
            disabled={committing}
          />
          <button
            className="commit-button"
            onClick={onCommit}
            disabled={!canCommit}
            title={
              !canCommit
                ? "Need a message and at least 1 staged file"
                : `Commit ${stagedCount} file${stagedCount === 1 ? "" : "s"}`
            }
          >
            {committing ? "Committing…" : `Commit ${stagedCount > 0 ? `(${stagedCount})` : ""}`}
          </button>
        </div>
      </div>

      <div className="changes-diff">
        {selected ? <DiffViewer diff={diff} /> : <div className="placeholder">Select a file.</div>}
      </div>
    </div>
  );
}

function ChangeGroup({ title, changes, selected, onSelect, action, onRevert }: {
  title: string;
  changes: GitFileChange[];
  selected: string | null;
  onSelect: (path: string) => void;
  action: { label: string; title: string; run: (path: string) => void };
  onRevert: (path: string) => void;
}) {
  return (
    <div className="change-group">
      <div className="change-group-header">{title}</div>
      {changes.map((c) => (
        <ChangeRow
          key={c.path}
          change={c}
          active={c.path === selected}
          actionLabel={action.label}
          actionTitle={action.title}
          onClick={() => onSelect(c.path)}
          onAction={() => action.run(c.path)}
          onRevert={() => onRevert(c.path)}
        />
      ))}
    </div>
  );
}

function ChangeRow({ change, active, actionLabel, actionTitle, onClick, onAction, onRevert }: {
  change: GitFileChange;
  active: boolean;
  actionLabel: string;
  actionTitle: string;
  onClick: () => void;
  onAction: () => void;
  onRevert: () => void;
}) {
  const badge = badgeFor(change);
  return (
    <div className={`change-row ${active ? "active" : ""}`} onClick={onClick} title={change.code}>
      <button
        className="change-action"
        onClick={(e) => { e.stopPropagation(); onAction(); }}
        title={actionTitle}
      >{actionLabel}</button>
      <span className={`change-badge ${badge.cls}`}>{badge.label}</span>
      <span className="change-path">{change.path}</span>
      <button
        className="change-revert"
        onClick={(e) => { e.stopPropagation(); onRevert(); }}
        title="Discard changes"
      >↺</button>
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
