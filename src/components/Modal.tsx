import { useRef, type ReactNode } from "react";
import { Dialog } from "@base-ui/react/dialog";

interface Props {
  onClose: () => void;
  /// Extra class for the panel (e.g. "shortcuts-modal"), appended to "modal":
  /// the per-modal width/layout rules keep applying.
  className?: string;
  /// Accessible name of the dialog.
  ariaLabel?: string;
  children: ReactNode;
}

/// Shared dialog shell for the "click-outside / Escape to close" modals
/// (About, Shortcuts, Getting Started, Modules, Wizard, worktree dialogs).
/// Base UI provides what the hand-rolled version lacked: focus trapped
/// inside, restored to the opener on close, the page behind made inert.
/// Initial focus goes to the first field or an explicit [data-autofocus]
/// target, else the panel itself.
///
/// NOTE: not for ApprovalModal — that one intentionally does NOT close on
/// backdrop click and runs a bespoke Escape handler, so it stays standalone.
export function Modal({ onClose, className, ariaLabel, children }: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const initialFocus = () =>
    (panel.current?.querySelector<HTMLElement>("[data-autofocus], input, textarea, select") ??
      panel.current) as HTMLElement | null;
  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Portal>
        <Dialog.Backdrop className="modal-backdrop" />
        <Dialog.Popup
          ref={panel}
          initialFocus={initialFocus}
          aria-label={ariaLabel}
          className={`modal${className ? ` ${className}` : ""}`}
        >
          {children}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
