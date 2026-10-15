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
  /// Column widths in px (list pane, inspector), drag-resizable, persisted.
  listWidth: number;
  inspectorWidth: number;
  setWidth: (col: "list" | "inspector", px: number | null) => void;
  /// Turn selected in the ledger minimap (Proof destination); not persisted.
  selectedTurnId: string | null;
  selectTurn: (id: string | null) => void;
  goTo: (d: Destination) => void;
  toggleList: () => void;
  toggleDrawer: () => void;
}

export const COLUMN_DEFAULTS = { list: 240, inspector: 360 } as const;
export const COLUMN_LIMITS = { list: [200, 420], inspector: [280, 560] } as const;

type Persisted = Pick<
  ShellState,
  "destination" | "lastTab" | "listOpen" | "drawerOpen" | "listWidth" | "inspectorWidth"
>;

function load(): Persisted {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const p = JSON.parse(raw) as Partial<ShellState>;
      return {
        destination: p.destination ?? "inbox",
        lastTab: p.lastTab ?? {},
        listOpen: p.listOpen ?? true,
        drawerOpen: p.drawerOpen ?? false,
        listWidth: p.listWidth ?? COLUMN_DEFAULTS.list,
        inspectorWidth: p.inspectorWidth ?? COLUMN_DEFAULTS.inspector,
      };
    }
  } catch {
    /* ignore */
  }
  return {
    destination: "inbox",
    lastTab: {},
    listOpen: true,
    drawerOpen: false,
    listWidth: COLUMN_DEFAULTS.list,
    inspectorWidth: COLUMN_DEFAULTS.inspector,
  };
}

function save(s: Persisted) {
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
  selectedTurnId: null,
  selectTurn: (selectedTurnId) => set({ selectedTurnId }),
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
  setWidth: (col, px) => {
    const [min, max] = COLUMN_LIMITS[col];
    const w = px === null ? COLUMN_DEFAULTS[col] : Math.max(min, Math.min(max, Math.round(px)));
    set(col === "list" ? { listWidth: w } : { inspectorWidth: w });
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
    save({ ...st, destination: dest, lastTab });
  });
}
