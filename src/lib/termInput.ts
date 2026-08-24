import type { TermInputDetail } from "../components/Terminal";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { useToastStore } from "../stores/toastStore";

/// Type text into the active session's agent input, switching to the terminal
/// tab. `submit: true` appends the Enter that sends it; default is the
/// review-first contract used everywhere (Jira seed, notes): the text lands in
/// the input and the human sends it. Returns false when there's no live
/// session to receive it (the text is copied to the clipboard instead).
///
/// Delivery goes through the `ac:term-input` window event — the same path the
/// model pill, voice PTT and drag-and-drop use: the Terminal owning the session
/// writes into its own PTY. Calling `ipc.termWrite` directly from here is a
/// bug: the registry keys PTYs by their spawn UUID, not by the session id this
/// module has access to.
export async function typeIntoActiveSession(
  text: string,
  opts: { submit?: boolean } = {},
): Promise<boolean> {
  const trimmed = text.replace(/\s+$/, "");
  if (!trimmed) return false;
  const { activeId, sessions } = useTerminalsStore.getState();
  useUIStore.getState().setTab("terminal");
  // A stopped session (its PTY exited) can't receive input — same fallback as
  // having no session at all, instead of writing into a dead terminal.
  const active = sessions.find((s) => s.id === activeId);
  if (!active || active.status !== "live") {
    try {
      await navigator.clipboard.writeText(trimmed);
    } catch {
      /* ignore */
    }
    useToastStore.getState().show("No live session — text copied instead", "info");
    return false;
  }
  const detail: TermInputDetail = {
    sessionId: active.id,
    data: trimmed + (opts.submit ? "\r" : ""),
  };
  window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
  return true;
}
