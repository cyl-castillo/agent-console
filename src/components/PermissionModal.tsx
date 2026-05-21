import { useEffect } from "react";

import { useChatStore } from "../stores/chatStore";
import type { PermissionRequest } from "../types/domain";

export function PermissionModal() {
  const pending = useChatStore((s) => s.pendingPermissions);
  const decide = useChatStore((s) => s.decidePermission);
  const approveAll = useChatStore((s) => s.approveAll);
  const setApproveAll = useChatStore((s) => s.setApproveAll);

  const req = pending[0];

  useEffect(() => {
    if (!req) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { decide(false); }
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { decide(true); }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [req, decide]);

  if (!req) return null;

  return (
    <div className="modal-backdrop" onClick={(e) => e.stopPropagation()}>
      <div className="modal perm-modal">
        <div className="modal-title">Approve agent action?</div>
        <div className="perm-tool">{req.toolName}</div>
        <pre className="perm-input">{formatInput(req.toolInput)}</pre>
        {pending.length > 1 && (
          <div className="field-hint">+{pending.length - 1} more queued</div>
        )}

        <label className="perm-approve-all">
          <input
            type="checkbox"
            checked={approveAll}
            onChange={(e) => setApproveAll(e.target.checked)}
          />
          Approve all actions for this session
        </label>

        <div className="modal-actions">
          <button onClick={() => decide(false)}>Deny (Esc)</button>
          <button className="primary" onClick={() => decide(true)}>Approve (Ctrl+Enter)</button>
        </div>
      </div>
    </div>
  );
}

function formatInput(input: unknown): string {
  if (input == null) return "";
  if (typeof input === "string") return input;
  try {
    return JSON.stringify(input, null, 2);
  } catch {
    return String(input);
  }
}

// Helper export so other components can re-use; not strictly needed for this file.
export type { PermissionRequest };
