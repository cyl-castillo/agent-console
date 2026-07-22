import { profileFor, type AgentKind } from "../agents/profiles";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";

/// Does an error message smell like broken engine authentication? Matches the
/// backend's exit_error hint plus the raw phrases claude/codex print.
export function isAuthError(message: string | null | undefined): boolean {
  if (!message) return false;
  return /log in again|authenticate|oauth|logged in|login/i.test(message);
}

/// One-click login repair: open a terminal session that runs the engine's
/// interactive login flow (claude / codex login). The OAuth dance needs a
/// browser and a human — the app's job is putting you one click away from it.
export function startLoginSession(agent: AgentKind): void {
  const project = useSessionStore.getState().project;
  const toast = useToastStore.getState();
  if (!project) {
    toast.show("Open a project first", "error");
    return;
  }
  const profile = profileFor(agent);
  const terminals = useTerminalsStore.getState();
  terminals.add(
    project.root,
    `${profile.binName} login`,
    undefined,
    agent,
    undefined,
    undefined,
    undefined,
    true,
  );
  window.dispatchEvent(new CustomEvent("ac:open-tab", { detail: "terminal" }));
  void terminals.persist();
  toast.show(`Complete the ${profile.label} login in the new terminal`, "info");
}
