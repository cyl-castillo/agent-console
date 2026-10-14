import { create } from "zustand";
import { isWorkbenchTab, type WorkbenchTab } from "../lib/workbenchTabs";
import { isTabEnabled, firstEnabledTab, useModulesStore } from "./modulesStore";
import { useOnboardingStore } from "./onboardingStore";

const KEY = "agent-console:workbench-tab:";

/// The active workbench tab, remembered per project. Turns by default: the
/// first thing a user sees next to the terminal is what the agent is doing,
/// structured — not its playbook.
interface WorkbenchState {
  tab: WorkbenchTab;
  /// The project the tab is persisted under (null before a project opens).
  projectRoot: string | null;
  setTab: (t: WorkbenchTab) => void;
  /// Called when a project opens: restore its last tab if still enabled.
  restoreFor: (projectRoot: string | null) => void;
}

export const useWorkbenchStore = create<WorkbenchState>((set, get) => ({
  tab: "turns",
  projectRoot: null,
  setTab: (tab) => {
    set({ tab });
    const root = get().projectRoot;
    if (root) {
      try {
        localStorage.setItem(`${KEY}${root}`, tab);
      } catch {
        /* ignore */
      }
    }
  },
  restoreFor: (projectRoot) => {
    set({ projectRoot });
    if (!projectRoot) return;
    try {
      const saved = localStorage.getItem(`${KEY}${projectRoot}`);
      if (isWorkbenchTab(saved) && isTabEnabled(saved)) set({ tab: saved });
    } catch {
      /* ignore */
    }
  },
}));

/// Cross-cutting reactions that used to live in App effects:
/// - when the module of the CURRENTLY OPEN tab gets switched off, bounce to
///   the first enabled group (keyed on the disabled set only: deep links that
///   land on a disabled tab must not be fought — the explicit click wins);
/// - the palette's "open workbench tab" event (tabs of a switched-off module
///   are unreachable from the palette; the Modules modal brings them back);
/// - mark the permissions tab visited as soon as the user opens it.
export function attachWorkbenchListeners(): () => void {
  const bounce = () => {
    const { tab } = useWorkbenchStore.getState();
    if (!isTabEnabled(tab)) useWorkbenchStore.setState({ tab: firstEnabledTab() });
  };
  bounce();
  const offModules = useModulesStore.subscribe((s, prev) => {
    if (s.disabled !== prev.disabled) bounce();
  });
  const offTab = useWorkbenchStore.subscribe((s, prev) => {
    if (s.tab !== prev.tab && s.tab === "permissions") {
      useOnboardingStore.getState().markVisitedPermissions();
    }
  });
  const onOpenWb = (e: Event) => {
    const d = (e as CustomEvent).detail;
    if (isWorkbenchTab(d) && isTabEnabled(d)) useWorkbenchStore.getState().setTab(d);
  };
  window.addEventListener("ac:open-workbench-tab", onOpenWb);
  return () => {
    offModules();
    offTab();
    window.removeEventListener("ac:open-workbench-tab", onOpenWb);
  };
}
