import { useEffect, useMemo, useRef, useState } from "react";

import { useChangesStore } from "../stores/changesStore";
import { DiffViewer } from "./DiffViewer";
import { SideBySideDiffViewer } from "./SideBySideDiffViewer";
import { ChangeTree } from "./ChangeTree";
import { BranchSwitcher } from "./BranchSwitcher";

const SUBJECT_LIMIT = 72;

export function ChangesView() {
  const {
    status,
    selected,
    diff,
    error,
    commitMessage,
    committing,
    recentMessages,
    refresh,
    setSelected,
    setCommitMessage,
    stage,
    unstage,
    stageMany,
    unstageMany,
    revert,
    revertAll,
    commit,
    loadCommitHistory,
    loadHeadMessage,
  } = useChangesStore();

  const [diffMode, setDiffMode] = useState<"unified" | "split">("split");
  const [amend, setAmend] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const commitRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const onFocus = () => commitRef.current?.focus();
    window.addEventListener("ac:focus-commit", onFocus);
    return () => window.removeEventListener("ac:focus-commit", onFocus);
  }, []);

  useEffect(() => {
    if (!status) refresh();
    loadCommitHistory();
  }, [status, refresh, loadCommitHistory]);

  // When the user toggles Amend on and the box is empty, prefill with HEAD msg.
  useEffect(() => {
    if (!amend) return;
    if (commitMessage.trim().length > 0) return;
    loadHeadMessage().then((m) => {
      if (m && useChangesStore.getState().commitMessage.trim().length === 0) {
        setCommitMessage(m);
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [amend]);

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

  // Reverting is irreversible, but in two different ways: a tracked file is
  // reset to its last commit (uncommitted edits lost), while an UNTRACKED file
  // is deleted from disk outright (git can't bring it back). Make the confirm
  // honest about which one is about to happen.
  const confirmRevert = (path: string) => {
    const change = allChanges.find((c) => c.path === path);
    const msg = change?.untracked
      ? `Delete the new file "${path}"?\n\n` +
        `It isn't tracked by git, so this permanently removes it from disk and can't be undone.`
      : `Discard uncommitted changes in "${path}"?\n\n` +
        `The file returns to its last committed state. This can't be undone.`;
    if (confirm(msg)) revert(path);
  };

  const confirmRevertAll = () => {
    const untracked = allChanges.filter((c) => c.untracked).length;
    const tracked = allChanges.length - untracked;
    const lines = [`Discard all ${allChanges.length} change(s)?`, ""];
    if (tracked) lines.push(`• Revert ${tracked} edited file(s) to their last commit`);
    if (untracked) lines.push(`• Permanently DELETE ${untracked} new file(s) from disk`);
    lines.push("", "This can't be undone.");
    if (confirm(lines.join("\n"))) revertAll();
  };

  const canCommit = (amend || stagedCount > 0) && commitMessage.trim().length > 0 && !committing;
  const subjectLine = commitMessage.split("\n", 1)[0] ?? "";
  const subjectOver = subjectLine.length > SUBJECT_LIMIT;

  const onCommit = async () => {
    const sha = await commit({ amend });
    if (sha) setAmend(false);
  };

  return (
    <div className="changes">
      <div className="changes-list">
        <div className="changes-list-header">
          <div className="changes-list-header-left">
            <BranchSwitcher currentBranch={status.branch} />
            <span className="changes-count">
              · {allChanges.length} change{allChanges.length === 1 ? "" : "s"}
            </span>
          </div>
          <span style={{ display: "flex", gap: 4 }}>
            <button onClick={refresh} title="Refresh status">
              ↻
            </button>
            {allChanges.length > 0 && (
              <button onClick={confirmRevertAll} title="Discard all changes">
                Discard all
              </button>
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
              <div className="change-group">
                <div className="change-group-header">
                  <span>Staged ({staged.length})</span>
                  <button
                    className="change-group-action"
                    onClick={() => unstageMany(staged.map((c) => c.path))}
                    title="Unstage all"
                  >
                    −
                  </button>
                </div>
                <ChangeTree
                  changes={staged}
                  selected={selected}
                  onSelect={setSelected}
                  fileAction={{ label: "−", title: "Unstage", run: unstage }}
                  folderAction={{ title: "Unstage folder", run: unstageMany }}
                  onRevert={confirmRevert}
                />
              </div>
            )}
            {unstaged.length > 0 && (
              <div className="change-group">
                <div className="change-group-header">
                  <span>Changes ({unstaged.length})</span>
                  <button
                    className="change-group-action"
                    onClick={() => stageMany(unstaged.map((c) => c.path))}
                    title="Stage all"
                  >
                    +
                  </button>
                </div>
                <ChangeTree
                  changes={unstaged}
                  selected={selected}
                  onSelect={setSelected}
                  fileAction={{ label: "+", title: "Stage", run: stage }}
                  folderAction={{ title: "Stage folder", run: stageMany }}
                  onRevert={confirmRevert}
                />
              </div>
            )}
          </>
        )}

        <div className="commit-box">
          <div className="commit-controls">
            <label
              className="commit-toggle"
              title="Replace the latest commit with these staged changes + this message"
            >
              <input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} />
              <span>Amend</span>
            </label>
            <span className="spacer" />
            <button
              className="commit-history-btn"
              onClick={() => setShowHistory((v) => !v)}
              disabled={recentMessages.length === 0}
              title="Pick from recent commit messages"
            >
              History ▾
            </button>
          </div>

          {showHistory && recentMessages.length > 0 && (
            <ul className="commit-history">
              {recentMessages.map((m, i) => (
                <li
                  key={i}
                  className="commit-history-item"
                  title={m}
                  onClick={() => {
                    setCommitMessage(m);
                    setShowHistory(false);
                  }}
                >
                  {firstLine(m)}
                </li>
              ))}
            </ul>
          )}

          <textarea
            ref={commitRef}
            className="commit-message"
            placeholder={
              amend
                ? "Edit the latest commit message…"
                : stagedCount === 0
                  ? "Stage at least one file to commit…"
                  : "Commit message (summary on first line)"
            }
            value={commitMessage}
            onChange={(e) => setCommitMessage(e.target.value)}
            rows={3}
            disabled={committing}
          />

          <div className="commit-meta">
            <span className={`commit-counter ${subjectOver ? "over" : ""}`}>
              {subjectLine.length}/{SUBJECT_LIMIT}
            </span>
            <span className="spacer" />
            <button
              className="btn btn-solid commit-button"
              onClick={onCommit}
              disabled={!canCommit}
              title={
                !canCommit
                  ? amend
                    ? "Need a message"
                    : "Need a message and at least 1 staged file"
                  : amend
                    ? `Amend HEAD (${stagedCount} additional staged file${stagedCount === 1 ? "" : "s"})`
                    : `Commit ${stagedCount} file${stagedCount === 1 ? "" : "s"}`
              }
            >
              {committing
                ? "Committing…"
                : amend
                  ? "Amend"
                  : `Commit ${stagedCount > 0 ? `(${stagedCount})` : ""}`}
            </button>
          </div>
        </div>
      </div>

      <div className="changes-diff">
        <div className="diff-toolbar">
          <span className="spacer" />
          <div className="diff-mode-toggle">
            <button
              className={diffMode === "split" ? "active" : ""}
              onClick={() => setDiffMode("split")}
              title="Side-by-side"
            >
              split
            </button>
            <button
              className={diffMode === "unified" ? "active" : ""}
              onClick={() => setDiffMode("unified")}
              title="Unified diff"
            >
              unified
            </button>
          </div>
        </div>
        {selected ? (
          diffMode === "split" ? (
            <SideBySideDiffViewer diff={diff} />
          ) : (
            <DiffViewer diff={diff} />
          )
        ) : (
          <div className="placeholder">Select a file.</div>
        )}
      </div>
    </div>
  );
}

function firstLine(s: string): string {
  const i = s.indexOf("\n");
  const line = i >= 0 ? s.slice(0, i) : s;
  return line.length > 80 ? line.slice(0, 80) + "…" : line;
}
