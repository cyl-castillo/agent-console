import {
  allPaletteActionIds,
  availablePaletteActions,
  runPaletteAction,
} from "../stores/paletteStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useApprovalStore } from "../stores/approvalStore";
import { useShellStore } from "../stores/shellStore";
import { useFlagsStore } from "../stores/flagsStore";

declare const __APP_VERSION__: string;

export interface AgentBridge {
  version: string;
  /// Every action id (stable contract) and which are runnable right now.
  listActions: () => { id: string; label?: string; available: boolean }[];
  /// Run an action by id; false when unknown or unavailable.
  invoke: (id: string) => Promise<boolean>;
  /// A small, stable snapshot of where the console is.
  state: () => {
    project: string | null;
    sessions: { id: string; name: string; status: string; active: boolean }[];
    pendingDecisions: number;
    destination: string;
    uiV2: boolean;
  };
}

/// Every action id with its label and current availability.
export function bridgeActions(): { id: string; label?: string; available: boolean }[] {
  const available = new Map(availablePaletteActions().map((a) => [a.id, a.label]));
  return allPaletteActionIds().map((id) => ({
    id,
    label: available.get(id),
    available: available.has(id),
  }));
}

/// A small, stable snapshot of where the console is.
export function bridgeState() {
  const t = useTerminalsStore.getState();
  return {
    project: useSessionStore.getState().project?.root ?? null,
    sessions: t.sessions
      .filter((s) => !s.archived)
      .map((s) => ({ id: s.id, name: s.name, status: s.status, active: s.id === t.activeId })),
    pendingDecisions: useApprovalStore.getState().queue.length,
    destination: useShellStore.getState().destination,
    uiV2: useFlagsStore.getState().flags["ui.v2"],
  };
}

/// The console as something an agent can drive without a mouse: the same
/// action registry the palette and voice use, exposed on `window` for
/// Playwright, computer-use agents reading the page, and the console's own
/// MCP server (which relays these calls from the loopback listener). Read
/// side only on top of stores; every write goes through an action.
export function installAgentBridge(): AgentBridge {
  const bridge: AgentBridge = {
    version: typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.0.0",
    listActions: bridgeActions,
    invoke: (id) => runPaletteAction(id),
    state: bridgeState,
  };
  if (typeof window !== "undefined") {
    (window as unknown as { __agentConsole: AgentBridge }).__agentConsole = bridge;
  }
  return bridge;
}
