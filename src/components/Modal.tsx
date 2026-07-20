import { useEffect, useRef } from "react";
import type { ReactNode } from "react";

interface Props {
  onClose: () => void;
  /// Extra class for the inner panel (e.g. "shortcuts-modal"), appended to "modal".
  className?: string;
  /// Accessible label for the dialog (falls back to aria-labelledby in children).
  ariaLabel?: string;
  children: ReactNode;
}

/// Shared dialog shell for the simple "click-outside / Escape to close" modals
/// (About, Shortcuts, Getting Started). Encapsulates the exact behavior those
/// components hand-rolled identically: backdrop click closes, inner click is
/// stopped from bubbling, and Escape closes. Adds dialog semantics + initial
/// focus for keyboard/AT users without changing any existing close behavior.
///
/// NOTE: not for ApprovalModal — that one intentionally does NOT close on
/// backdrop click and runs a bespoke Escape handler, so it stays standalone.
export function Modal({ onClose, className, ariaLabel, children }: Props) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        ref={panelRef}
        className={`modal${className ? ` ${className}` : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}
