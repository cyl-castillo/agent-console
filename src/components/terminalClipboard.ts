/// Clipboard key-binding policy for the terminal.
///
/// xterm owns the mouse, so its selection is internal — the webview never sees
/// a native DOM selection, which means the browser/webview offers NO copy path
/// on its own (no context-menu Copy, and Ctrl+C goes to the PTY as SIGINT).
/// This helper decides, per keydown, whether we handle the key as a clipboard
/// action instead of letting xterm forward it to the PTY.
///
/// Bindings (terminal conventions):
/// - Ctrl/Cmd+Shift+C          → copy the selection (never reaches the PTY)
/// - Ctrl/Cmd+C with selection → copy (users expect select→Ctrl+C to copy;
///   the selection is cleared so pressing Ctrl+C again sends SIGINT as usual)
/// - Ctrl/Cmd+C without selection → passthrough (SIGINT — sacred)
/// - Ctrl/Cmd+Shift+V          → paste (complements the plain-paste event flow)

export type ClipboardAction = "copy" | "copy-and-clear" | "paste" | null;

export interface KeyLike {
  type: string;
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

export function clipboardActionFor(e: KeyLike, hasSelection: boolean): ClipboardAction {
  if (e.type !== "keydown") return null;
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) return null;
  const key = e.key.toLowerCase();
  if (key === "c") {
    if (e.shiftKey) return hasSelection ? "copy" : null;
    return hasSelection ? "copy-and-clear" : null;
  }
  if (key === "v" && e.shiftKey) return "paste";
  return null;
}
