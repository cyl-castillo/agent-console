import { homeDir } from "@tauri-apps/api/path";

import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";

/// One-click install, review-first: open a plain-shell session with the
/// official installer TYPED but not sent — the user reads the exact command,
/// can edit it, and presses Enter. Nothing ever runs behind their back.
///
/// Works before any project is open (the first-run case this exists for):
/// the session falls back to the user's home directory.
export async function startInstallSession(toolName: string, command: string): Promise<void> {
  const toast = useToastStore.getState();
  const project = useSessionStore.getState().project;
  let cwd = project?.root;
  if (!cwd) {
    try {
      cwd = await homeDir();
    } catch {
      toast.show("Couldn't resolve a directory for the install terminal", "error");
      return;
    }
  }
  const terminals = useTerminalsStore.getState();
  terminals.add(
    cwd,
    `install ${toolName}`,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    command,
  );
  window.dispatchEvent(new CustomEvent("ac:open-tab", { detail: "terminal" }));
  void terminals.persist();
  toast.show("Review the typed command, then press Enter to install", "info");
}
