import { useEffect, useRef, type ReactNode } from "react";
import { AlertDialog } from "@base-ui/react/alert-dialog";
import { cn } from "@/lib/cn";
import { Button } from "./Button";

/// Yes/no question. `danger` puts the destructive action on the right in
/// red AND moves initial focus to the safe button, so Enter never deletes.
export function ConfirmDialog({
  open,
  onResolve,
  title = "Are you sure?",
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  danger,
}: {
  open: boolean;
  onResolve: (confirmed: boolean) => void;
  title?: string;
  message: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const id = window.setTimeout(() => (danger ? cancelRef : confirmRef).current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [open, danger]);
  return (
    <AlertDialog.Root open={open} onOpenChange={(o) => !o && onResolve(false)}>
      <AlertDialog.Portal>
        <AlertDialog.Backdrop className="fixed inset-0 z-40 bg-black/50 data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-2" />
        <AlertDialog.Popup
          className={cn(
            "fixed left-1/2 top-1/2 z-50 w-[400px] max-w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2",
            "rounded-3 border border-border bg-surface-2 p-4 font-sans text-body text-foreground shadow-e3",
            "data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-2",
          )}
        >
          <AlertDialog.Title className="m-0 text-title font-semibold">{title}</AlertDialog.Title>
          <AlertDialog.Description className="mt-2 whitespace-pre-line text-body text-muted-foreground">
            {message}
          </AlertDialog.Description>
          <div className="mt-4 flex justify-end gap-2">
            <Button ref={cancelRef} variant="secondary" onClick={() => onResolve(false)}>
              {cancelLabel}
            </Button>
            <Button
              ref={confirmRef}
              variant={danger ? "danger" : "primary"}
              onClick={() => onResolve(true)}
            >
              {confirmLabel}
            </Button>
          </div>
        </AlertDialog.Popup>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}
