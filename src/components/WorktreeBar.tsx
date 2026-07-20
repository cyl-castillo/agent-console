import { useCallback, useEffect, useState } from "react";

import { useTerminalsStore } from "../stores/terminalsStore";
import { useChangesStore } from "../stores/changesStore";
import { useToastStore } from "../stores/toastStore";
import { ipc } from "../ipc/tauri";
import { Modal } from "./Modal";
import type { MergeOutcome, WorktreeStatusInfo } from "../types/domain";

/// Lifecycle bar for worktree sessions, shown above the terminal: where the
/// session branch stands vs its base, plus the two exits — merge back or
/// discard. Renders nothing for sessions that run in the project checkout.
export function WorktreeBar() {
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const closeSession = useTerminalsStore((s) => s.close);
  const refreshChanges = useChangesStore((s) => s.refresh);
  const show = useToastStore((s) => s.show);

  const session = sessions.find((s) => s.id === activeId);
  const wt = session?.worktree;

  const [st, setSt] = useState<WorktreeStatusInfo | null>(null);
  const [dialog, setDialog] = useState<"merge" | "discard" | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [conflicts, setConflicts] = useState<MergeOutcome | null>(null);
  const [deleteBranch, setDeleteBranch] = useState(true);

  const refresh = useCallback(() => {
    if (!wt) return;
    ipc
      .worktreeStatus(wt)
      .then(setSt)
      .catch(() => setSt(null));
  }, [wt]);

  useEffect(() => {
    setSt(null);
    if (!wt) return;
    refresh();
    const t = setInterval(refresh, 15_000);
    return () => clearInterval(t);
  }, [wt, refresh]);

  if (!session || !wt) return null;

  const closeDialog = () => {
    if (busy) return;
    setDialog(null);
    setErr(null);
    setConflicts(null);
  };

  const doMerge = async (deleteAfter: boolean) => {
    setBusy(true);
    setErr(null);
    setConflicts(null);
    try {
      const outcome = await ipc.worktreeMerge(wt, deleteAfter);
      if (!outcome.merged) {
        setConflicts(outcome);
        return;
      }
      show(`Merged ${wt.branch} into ${wt.baseBranch}`, "success");
      setDialog(null);
      if (deleteAfter) {
        await closeSession(session.id);
      } else {
        refresh();
      }
      refreshChanges();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const doDiscard = async () => {
    setBusy(true);
    setErr(null);
    try {
      await ipc.worktreeDiscard(wt, deleteBranch);
      show(
        deleteBranch
          ? `Discarded ${wt.branch} (worktree and branch deleted)`
          : `Worktree removed — branch ${wt.branch} kept`,
        "info",
      );
      setDialog(null);
      await closeSession(session.id);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stateLabel = st
    ? `${st.dirtyFiles} uncommitted · ${st.ahead} ahead${st.behind > 0 ? ` · ${st.behind} behind` : ""}`
    : "…";

  return (
    <>
      <div className="worktree-bar">
        <span className="wt-glyph" aria-hidden>
          ⎇
        </span>
        <span className="wt-branch" title={wt.path}>
          {wt.branch}
        </span>
        <span className="wt-base">→ {wt.baseBranch}</span>
        <span
          className="wt-state"
          title="Uncommitted files in the worktree · commits ahead of / behind the base branch"
        >
          {stateLabel}
        </span>
        <span className="spacer" />
        <button className="btn wt-merge-btn" onClick={() => setDialog("merge")}>
          Merge into {wt.baseBranch}
        </button>
        <button
          className="btn btn-ghost wt-discard-btn"
          onClick={() => {
            setDeleteBranch(true);
            setDialog("discard");
          }}
        >
          Discard
        </button>
      </div>

      {dialog === "merge" && (
        <Modal onClose={closeDialog} className="wt-modal" ariaLabel="Merge session worktree">
          <h3>
            Merge {wt.branch} into {wt.baseBranch}
          </h3>
          <p className="wt-modal-line">
            Uncommitted work in the worktree is committed first, then merged into{" "}
            <strong>{wt.baseBranch}</strong> in your main checkout (which must be on {wt.baseBranch}
            ). A conflicted merge is aborted — your checkout is left untouched.
          </p>
          {st && <p className="wt-modal-line wt-modal-meta">{stateLabel}</p>}
          {conflicts && (
            <div className="wt-modal-conflicts">
              <p>Merge conflicts — nothing was changed. Conflicting files:</p>
              <ul>
                {conflicts.conflictFiles.map((f) => (
                  <li key={f}>
                    <code>{f}</code>
                  </li>
                ))}
              </ul>
              <p className="wt-modal-meta">
                Resolve by updating {wt.baseBranch} or the session branch, then retry.
              </p>
            </div>
          )}
          {err && <p className="wt-modal-error">{err}</p>}
          <div className="wt-modal-actions">
            <button className="btn" disabled={busy} onClick={() => doMerge(false)}>
              Merge &amp; keep session
            </button>
            <button className="btn btn-primary" disabled={busy} onClick={() => doMerge(true)}>
              Merge &amp; clean up
            </button>
            <span className="spacer" />
            <button className="btn btn-ghost" disabled={busy} onClick={closeDialog}>
              Cancel
            </button>
          </div>
          <p className="wt-modal-meta">
            “Clean up” deletes the worktree checkout and the {wt.branch} branch after a successful
            merge, and closes this session.
          </p>
        </Modal>
      )}

      {dialog === "discard" && (
        <Modal onClose={closeDialog} className="wt-modal" ariaLabel="Discard session worktree">
          <h3>Discard {wt.branch}?</h3>
          <p className="wt-modal-line">
            Deletes the worktree checkout
            {st && st.dirtyFiles > 0 ? (
              <>
                {" "}
                —{" "}
                <strong>
                  {st.dirtyFiles} uncommitted file{st.dirtyFiles === 1 ? "" : "s"} will be lost
                </strong>
              </>
            ) : null}
            . The session terminal closes.
          </p>
          <label className="wt-modal-check">
            <input
              type="checkbox"
              checked={deleteBranch}
              onChange={(e) => setDeleteBranch(e.target.checked)}
            />
            <span>
              Also delete the <code>{wt.branch}</code> branch
              {st && st.ahead > 0 ? (
                <>
                  {" "}
                  —{" "}
                  <strong>
                    its {st.ahead} commit{st.ahead === 1 ? "" : "s"} are lost too
                  </strong>
                </>
              ) : null}
            </span>
          </label>
          {err && <p className="wt-modal-error">{err}</p>}
          <div className="wt-modal-actions">
            <button className="btn wt-danger" disabled={busy} onClick={doDiscard}>
              {deleteBranch ? "Discard worktree + branch" : "Discard worktree, keep branch"}
            </button>
            <span className="spacer" />
            <button className="btn btn-ghost" disabled={busy} onClick={closeDialog}>
              Cancel
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
