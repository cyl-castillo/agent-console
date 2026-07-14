import { ipc } from "../ipc/tauri";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { useToastStore } from "../stores/toastStore";

/// Type text into the active session's agent input, switching to the terminal
/// tab. `submit: true` appends the Enter that sends it; default is the
/// review-first contract used everywhere (Jira seed, notes): the text lands in
/// the input and the human sends it. Returns false when there's no live
/// session to receive it (the text is copied to the clipboard instead).
export async function typeIntoActiveSession(
  text: string,
  opts: { submit?: boolean } = {},
): Promise<boolean> {
  const trimmed = text.replace(/\s+$/, "");
  if (!trimmed) return false;
  const { activeId } = useTerminalsStore.getState();
  useUIStore.getState().setTab("terminal");
  if (!activeId) {
    try { await navigator.clipboard.writeText(trimmed); } catch { /* ignore */ }
    useToastStore.getState().show("No active session — text copied instead", "info");
    return false;
  }
  try {
    await ipc.termWrite(activeId, trimmed + (opts.submit ? "\r" : ""));
    return true;
  } catch {
    useToastStore.getState().show("Couldn't reach the session terminal", "error");
    return false;
  }
}
