import { ConfirmDialog as Confirm } from "../ui/ConfirmDialog";
import { useConfirmStore } from "../stores/confirmStore";

/// Renders the pending `confirmDialog()` request. Mounted once in OverlayHost.
/// Escape / backdrop = cancel; `danger` requests start with focus on the safe
/// button so Enter never destroys anything by habit.
export function ConfirmDialog() {
  const pending = useConfirmStore((s) => s.pending);
  const settle = useConfirmStore((s) => s.settle);
  if (!pending) return null;
  return (
    <Confirm
      open
      onResolve={settle}
      title={pending.title}
      message={pending.message}
      confirmLabel={pending.confirmLabel}
      cancelLabel={pending.cancelLabel}
      danger={pending.danger}
    />
  );
}
