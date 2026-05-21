import { useEffect } from "react";

import { useChatStore } from "../stores/chatStore";

/// Operational approval queue. Shows the full stack of pending operations
/// with batch approve/reject controls — not a single-shot popup.
export function PermissionModal() {
  const pending = useChatStore((s) => s.pendingPermissions);
  const decide = useChatStore((s) => s.decidePermission);
  const approveAll = useChatStore((s) => s.approveAll);
  const setApproveAll = useChatStore((s) => s.setApproveAll);

  useEffect(() => {
    if (pending.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") decide(false);
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) decide(true);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [pending.length, decide]);

  if (pending.length === 0) return null;

  const approveAllNow = async () => {
    while (useChatStore.getState().pendingPermissions.length > 0) {
      await decide(true);
    }
  };
  const rejectAllNow = async () => {
    while (useChatStore.getState().pendingPermissions.length > 0) {
      await decide(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={(e) => e.stopPropagation()}>
      <div className="modal perm-modal">
        <div className="modal-title">
          Pending operations
          <span className="perm-count">{pending.length}</span>
        </div>

        <ul className="perm-list">
          {pending.map((req, i) => (
            <li key={req.id} className={`perm-row ${i === 0 ? "perm-row-active" : ""}`}>
              <div className="perm-row-head">
                <span className="perm-tool">{req.toolName}</span>
                {i === 0 && <span className="perm-tag">next</span>}
              </div>
              <pre className="perm-input">{formatInput(req.toolInput)}</pre>
              {i === 0 && (
                <div className="perm-row-actions">
                  <button onClick={() => decide(false)}>deny (Esc)</button>
                  <button className="primary" onClick={() => decide(true)}>approve (Ctrl+Enter)</button>
                </div>
              )}
            </li>
          ))}
        </ul>

        <label className="perm-approve-all">
          <input
            type="checkbox"
            checked={approveAll}
            onChange={(e) => setApproveAll(e.target.checked)}
          />
          approve every operation for the rest of this session
        </label>

        {pending.length > 1 && (
          <div className="modal-actions">
            <button onClick={rejectAllNow}>reject all ({pending.length})</button>
            <button className="primary" onClick={approveAllNow}>approve all ({pending.length})</button>
          </div>
        )}
      </div>
    </div>
  );
}

function formatInput(input: unknown): string {
  if (input == null) return "";
  if (typeof input === "string") return input;
  try { return JSON.stringify(input, null, 2); } catch { return String(input); }
}
