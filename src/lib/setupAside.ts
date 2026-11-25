import type { AgentKind } from "../agents/profiles";
import { useOverlaysStore } from "../stores/overlaysStore";
import { useToastStore } from "../stores/toastStore";

/// The first-run wizard is a modal: while it is open nothing behind it can be
/// reached, the terminal included. A step that opens a terminal (install,
/// login) makes the wizard step aside instead of leaving the terminal under
/// it: hidden, NOT finished, the engine it was setting up remembered, and a
/// toast that stays until the person comes back. Closing the wizard to reach
/// the terminal used to be the only way, and closing marks it done for good.
let aside: { engine: AgentKind | null; toast: string } | null = null;

export function setupStepsAside(engine: AgentKind | null): void {
  const toasts = useToastStore.getState();
  if (aside) toasts.dismiss(aside.toast);
  useOverlaysStore.getState().hide("wizard");
  const toast = toasts.show("Setup is waiting while you use the terminal", "info", {
    action: { label: "Back to setup", onClick: backToSetup },
    timeoutMs: 0,
  });
  aside = { engine, toast };
}

/// Reopen the wizard where it was. Also reachable from the palette
/// ("Run first-run setup wizard") when the toast was dismissed.
export function backToSetup(): void {
  useOverlaysStore.getState().show("wizard");
}

/// True from the moment the wizard stepped aside until it is open again: the
/// first-run auto-open must not put it back over the terminal meanwhile.
export function isSetupAside(): boolean {
  return aside !== null;
}

/// The engine the wizard was setting up when it stepped aside, if it did.
export function asideEngine(): AgentKind | null {
  return aside?.engine ?? null;
}

/// The wizard is open again: the way back is no longer needed.
export function setupResumed(): void {
  if (!aside) return;
  useToastStore.getState().dismiss(aside.toast);
  aside = null;
}
