import { create } from "zustand";
import { showToast, toastManager } from "../ui/Toast";
import { reportProblem } from "../lib/reportProblem";

export type ToastTone = "info" | "success" | "error";

export interface Toast {
  id: string;
  message: string;
  tone: ToastTone;
}

interface ToastState {
  toasts: Toast[];
  /// Returns the toast id. `action` adds a trailing button ("Undo");
  /// `timeoutMs` overrides the transient delay.
  show: (
    message: string,
    tone?: ToastTone,
    opts?: { action?: { label: string; onClick: () => void }; timeoutMs?: number },
  ) => string;
  dismiss: (id: string) => void;
}

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],

  show: (message, tone = "info", opts) => {
    // Rendered by the Base UI toast manager (ui/Toast → ToastHost). Info and
    // success are transient; errors persist until dismissed so a real
    // failure can't vanish before it's read — and carry a one-click field
    // report, the moment it is worth the most.
    const action =
      opts?.action ??
      (tone === "error"
        ? { label: "Report", onClick: () => void reportProblem(message) }
        : undefined);
    const id = showToast(message, tone, action ? { action } : undefined, opts?.timeoutMs);
    set((s) => ({ toasts: [...s.toasts, { id, message, tone }].slice(-4) }));
    if (tone !== "error" || opts?.timeoutMs) {
      window.setTimeout(() => get().dismiss(id), opts?.timeoutMs ?? 2600);
    }
    return id;
  },

  dismiss: (id) => {
    toastManager.close(id);
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
}));
