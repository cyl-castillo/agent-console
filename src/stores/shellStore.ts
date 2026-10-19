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
  /// Whether the terminal drawer under the conversation is open. Open by
  /// default (2026-10-19, Carlos): the CLI is where trust, login, approvals
  /// and the agent's words happen today; the rendered chat is the companion.
  drawerOpen: boolean;
  /// Drawer height as a percentage of the Sessions centre (DRAWER_LIMITS).
  drawerPct: number;
  /// Hide the rendered conversation so the terminal takes the whole centre.
  chatCollapsed: boolean;
  setDrawerPct: (pct: number | null) => void;
  toggleChat: () => void;
  /// Column widths in px (list pane, inspector), drag-resizable, persisted.
  listWidth: number;
  inspectorWidth: number;
  setWidth: (col: "list" | "inspector", px: number | null) => void;
  /// The new-session chooser (the "+" and Ctrl+T open the same one); not persisted.
  chooserOpen: boolean;
  setChooserOpen: (v: boolean) => void;
  /// Turn selected in the ledger minimap (Proof destination); not persisted.
  selectedTurnId: string | null;
  selectTurn: (id: string | null) => void;
  goTo: (d: Destination) => void;
  toggleList: () => void;
  toggleDrawer: () => void;
}

export const COLUMN_DEFAULTS = { list: 240, inspector: 360 } as const;
export const DRAWER_DEFAULT_PCT = 60;
export const DRAWER_LIMITS = [25, 90] as const;
export const COLUMN_LIMITS = { list: [200, 420], inspector: [280, 560] } as const;

type Persisted = Pick<
  ShellState,
  | "destination"
  | "lastTab"
  | "listOpen"
  | "drawerOpen"
  | "drawerPct"
  | "chatCollapsed"
  | "listWidth"
  | "inspectorWidth"
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
        drawerOpen: p.drawerOpen ?? true,
        drawerPct: p.drawerPct ?? DRAWER_DEFAULT_PCT,
        chatCollapsed: p.chatCollapsed ?? false,
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
    drawerOpen: true,
    drawerPct: DRAWER_DEFAULT_PCT,
    chatCollapsed: false,
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
  chooserOpen: false,
  setChooserOpen: (chooserOpen) => set({ chooserOpen }),
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
  setDrawerPct: (pct) => {
    const [min, max] = DRAWER_LIMITS;
    const v = pct === null ? DRAWER_DEFAULT_PCT : Math.max(min, Math.min(max, Math.round(pct)));
    set({ drawerPct: v });
    save(get());
  },
  toggleChat: () => {
    // Hiding the chat means the terminal is the stage: make sure it shows.
    set((s) => ({
      chatCollapsed: !s.chatCollapsed,
      drawerOpen: s.chatCollapsed ? s.drawerOpen : true,
    }));
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
  const follow = (tab: WorkbenchTab) => {
    const dest = destinationForTab(tab);
    const st = useShellStore.getState();
    const lastTab = { ...st.lastTab, [dest]: tab };
    useShellStore.setState({ destination: dest, lastTab });
    save({ ...st, destination: dest, lastTab });
  };
  const unsub = useWorkbenchStore.subscribe((s, prev) => {
    if (s.tab !== prev.tab) follow(s.tab);
  });
  // A deep link to the tab already selected still has to pull the shell to
  // its destination (e.g. "Open Turns" from the Inbox with Turns remembered).
  const onOpen = (e: Event) => {
    const tab = (e as CustomEvent<WorkbenchTab>).detail;
    if (tab && isTabEnabled(tab)) follow(tab);
  };
  window.addEventListener("ac:open-workbench-tab", onOpen);
  return () => {
    unsub();
    window.removeEventListener("ac:open-workbench-tab", onOpen);
  };
}
