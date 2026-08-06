import { profileFor, type AgentKind, type AgentProfile } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";

/// Does an error message smell like broken engine authentication? Matches the
/// backend's exit_error hint plus the raw phrases claude/codex print.
export function isAuthError(message: string | null | undefined): boolean {
  if (!message) return false;
  return /log in again|authenticate|oauth|logged in|login/i.test(message);
}

/// Which command the "fix login" terminal should run for this agent. Prefers
/// the profile's modern command, but only once the backend probe proves the
/// installed CLI understands it: `claude auth status` answering at all means
/// the `auth` subcommand exists (2.1.41+). An unknown answer — old CLI, binary
/// missing, probe failed — keeps the command that works everywhere.
export async function resolveLoginCmd(profile: AgentProfile): Promise<string> {
  if (!profile.modernLoginCmd || profile.kind !== "claude") return profile.loginCmd;
  try {
    const status = await ipc.claudeAuthStatus();
    return status ? profile.modernLoginCmd : profile.loginCmd;
  } catch {
    return profile.loginCmd;
  }
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
