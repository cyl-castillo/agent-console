import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";

/// One-click install, review-first: open a plain-shell session with the
/// official installer TYPED but not sent — the user reads the exact command,
/// can edit it, and presses Enter. Nothing ever runs behind their back.
///
/// Needs an open project: terminals only render inside one, so a session
/// started without it would hold the typed command where nobody can see it.
/// Answers whether a terminal was opened, so a dialog that covers the
/// terminal knows to get out of the way.
export async function startInstallSession(toolName: string, command: string): Promise<boolean> {
  const toast = useToastStore.getState();
  const project = useSessionStore.getState().project;
  if (!project) {
    toast.show("Open a project folder first: the install terminal opens inside it", "info");
    return false;
  }
  const terminals = useTerminalsStore.getState();
  terminals.add(
    project.root,
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
  return true;
}
