// Shell actions shared by the top bar, the empty states, the palette's window
// events and voice commands. They read the stores at call time so a listener
// registered once stays correct across project switches.
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";

export function newSession(): void {
  const project = useSessionStore.getState().project;
  if (!project) return;
  const t = useTerminalsStore.getState();
  t.add(project.root);
  useUIStore.getState().setTab("terminal");
  void t.persist();
}

export function copyProjectPath(): void {
  const project = useSessionStore.getState().project;
  if (!project) return;
  const show = useToastStore.getState().show;
  navigator.clipboard
    ?.writeText(project.root)
    .then(() => show("Project path copied", "success"))
    .catch(() => show("Could not copy project path", "error"));
}
