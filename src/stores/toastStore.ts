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
  show: (message: string, tone?: ToastTone) => void;
  dismiss: (id: string) => void;
}

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],

  show: (message, tone = "info") => {
    // Rendered by the Base UI toast manager (ui/Toast → ToastHost). Info and
    // success are transient; errors persist until dismissed so a real
    // failure can't vanish before it's read — and carry a one-click field
    // report, the moment it is worth the most.
    const id = showToast(
      message,
      tone,
      tone === "error"
        ? { action: { label: "Report", onClick: () => void reportProblem(message) } }
        : undefined,
    );
    set((s) => ({ toasts: [...s.toasts, { id, message, tone }].slice(-4) }));
    if (tone !== "error") {
      window.setTimeout(() => get().dismiss(id), 2600);
    }
  },

  dismiss: (id) => {
    toastManager.close(id);
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
}));
