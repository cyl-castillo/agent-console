import { useEffect, useRef } from "react";

import { Modal } from "./Modal";
import { useConfirmStore } from "../stores/confirmStore";

/// Renders the pending `confirmDialog()` request. Mounted once in App.
/// Escape / backdrop = cancel (Modal's onClose); Enter = the focused button,
/// which starts on the affirmative one like the native dialog it replaces.
export function ConfirmDialog() {
  const pending = useConfirmStore((s) => s.pending);
  const settle = useConfirmStore((s) => s.settle);
  const okRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (pending) okRef.current?.focus();
  }, [pending]);

  if (!pending) return null;

  return (
    <Modal onClose={() => settle(false)} className="confirm-modal" ariaLabel={pending.title}>
      <div className="confirm-title">{pending.title ?? "Are you sure?"}</div>
      <div className="confirm-message">{pending.message}</div>
      <div className="modal-actions">
        <button type="button" onClick={() => settle(false)}>
          {pending.cancelLabel ?? "Cancel"}
        </button>
        <button
          type="button"
          ref={okRef}
          className={pending.danger ? "btn-danger" : "btn-primary"}
          onClick={() => settle(true)}
        >
          {pending.confirmLabel ?? "Confirm"}
        </button>
      </div>
    </Modal>
  );
}
