import { create } from "zustand";

/// In-app confirmation, replacing `window.confirm()` / `window.alert()`.
///
/// The native dialogs are a trap inside a Tauri webview: WebKitGTK can skip
/// the dialog and return without ever asking (the lesson ProofPanel and the
/// terminal-links handler already learned), and a phantom "yes" on "delete
/// this session" or "discard all changes" is not an acceptable failure mode.
/// They also block the whole UI thread, look foreign, and can't be styled to
/// say *how* destructive the action is.
///
/// `confirmDialog()` is a plain async function so stores, hooks and components
/// can all await it; `ConfirmDialog` (components/) renders whatever is pending.
/// Requests queue: a second ask while one is open waits its turn instead of
/// clobbering the first.

export interface ConfirmRequest {
  /// Short heading; defaults to "Are you sure?".
  title?: string;
  /// Body text. Newlines are preserved.
  message: string;
  /// Label of the affirmative button; defaults to "Confirm".
  confirmLabel?: string;
  /// Label of the negative button; defaults to "Cancel".
  cancelLabel?: string;
  /// Style the affirmative button as destructive.
  danger?: boolean;
}

interface Pending extends ConfirmRequest {
  id: string;
  resolve: (ok: boolean) => void;
}

interface ConfirmState {
  /// The request currently shown (head of the queue), or null.
  pending: Pending | null;
  queue: Pending[];
  ask: (req: ConfirmRequest) => Promise<boolean>;
  /// Answer the pending request; the next queued one (if any) becomes pending.
  settle: (ok: boolean) => void;
}

let seq = 0;

export const useConfirmStore = create<ConfirmState>((set, get) => ({
  pending: null,
  queue: [],

  ask: (req) =>
    new Promise<boolean>((resolve) => {
      const item: Pending = { ...req, id: `confirm-${++seq}`, resolve };
      const { pending } = get();
      if (pending) set((s) => ({ queue: [...s.queue, item] }));
      else set({ pending: item });
    }),

  settle: (ok) => {
    const { pending, queue } = get();
    if (!pending) return;
    const [next, ...rest] = queue;
    set({ pending: next ?? null, queue: rest });
    pending.resolve(ok);
  },
}));

/// Ask the user. A bare string is the message with defaults; pass an object
/// to set the title/labels or mark the action destructive.
export function confirmDialog(req: ConfirmRequest | string): Promise<boolean> {
  return useConfirmStore.getState().ask(typeof req === "string" ? { message: req } : req);
}

/// Component-flavoured alias so call sites read like the hook they replace.
export function useConfirm(): typeof confirmDialog {
  return confirmDialog;
}
