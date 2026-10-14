import { create } from "zustand";
import { isTabEnabled } from "./modulesStore";
import { useWorkbenchStore } from "./workbenchStore";
import {
  destinationForTab,
  firstTabOf,
  destinationDef,
  type Destination,
} from "../app/v2/destinations";
import type { WorkbenchTab } from "../lib/workbenchTabs";

const KEY = "agent-console:shell-v2";

interface ShellState {
  destination: Destination;
  /// Last tab opened inside each destination, so going back lands where you left.
  lastTab: Partial<Record<Destination, WorkbenchTab>>;
  /// Whether the list pane (second column) is open.
  listOpen: boolean;
  /// Whether the terminal drawer under the conversation is open.
  drawerOpen: boolean;
  goTo: (d: Destination) => void;
  toggleList: () => void;
  toggleDrawer: () => void;
}

function load(): Pick<ShellState, "destination" | "lastTab" | "listOpen" | "drawerOpen"> {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const p = JSON.parse(raw) as Partial<ShellState>;
      return {
        destination: p.destination ?? "inbox",
        lastTab: p.lastTab ?? {},
        listOpen: p.listOpen ?? true,
        drawerOpen: p.drawerOpen ?? false,
      };
    }
  } catch {
    /* ignore */
  }
  return { destination: "inbox", lastTab: {}, listOpen: true, drawerOpen: false };
}

function save(s: Pick<ShellState, "destination" | "lastTab" | "listOpen" | "drawerOpen">) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* ignore */
  }
}

/// Where the v2 shell is. The workbench tab stays the routing currency: picking
/// a destination opens its last (or first enabled) tab, and a tab opened by
/// the palette or a deep link pulls the shell to that tab's destination.
export const useShellStore = create<ShellState>((set, get) => ({
  ...load(),
  goTo: (destination) => {
    const def = destinationDef(destination);
    if (def.sections.length > 0) {
      const remembered = get().lastTab[destination];
      const tab =
        remembered && isTabEnabled(remembered)
          ? remembered
          : (def.sections.flatMap((s) => s.tabs).find(isTabEnabled) ?? firstTabOf(destination));
      if (tab) useWorkbenchStore.getState().setTab(tab);
    }
    set({ destination });
    save(get());
  },
  toggleList: () => {
    set((s) => ({ listOpen: !s.listOpen }));
    save(get());
  },
  toggleDrawer: () => {
    set((s) => ({ drawerOpen: !s.drawerOpen }));
    save(get());
  },
}));

/// Keep destination ↔ tab coherent when the tab changes from outside (palette,
/// Getting Started deep links, onOpenRoom).
export function attachShellListeners(): () => void {
  return useWorkbenchStore.subscribe((s, prev) => {
    if (s.tab === prev.tab) return;
    const dest = destinationForTab(s.tab);
    const st = useShellStore.getState();
    const lastTab = { ...st.lastTab, [dest]: s.tab };
    useShellStore.setState({ destination: dest, lastTab });
    save({ destination: dest, lastTab, listOpen: st.listOpen, drawerOpen: st.drawerOpen });
  });
}
