import { create } from "zustand";

import { MODULES, moduleForTab } from "../lib/modules";
import { WORKBENCH_GROUPS, type WorkbenchGroupKey, type WorkbenchTab } from "../lib/workbenchTabs";

const STORAGE_KEY = "agent-console.modules.v1";
/// Experimental modules are born off, so for them the ENABLED set is what
/// persists: the ones the user switched on.
const OPT_IN_KEY = "agent-console.modules.optin.v1";

const isExperimental = (k: WorkbenchGroupKey) => MODULES.some((m) => m.key === k && m.experimental);

function loadOptIn(): WorkbenchGroupKey[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(OPT_IN_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (k): k is WorkbenchGroupKey =>
        typeof k === "string" && isExperimental(k as WorkbenchGroupKey),
    );
  } catch {
    return [];
  }
}

/// What is hidden right now: what the user switched off, plus every
/// experimental module they have not switched on.
function effective(off: WorkbenchGroupKey[], optIn: WorkbenchGroupKey[]): WorkbenchGroupKey[] {
  const stay = off.filter((k) => !isExperimental(k));
  const dormant = MODULES.filter((m) => m.experimental && !optIn.includes(m.key)).map((m) => m.key);
  return [...stay, ...dormant];
}

/// The DISABLED set is what persists (not the enabled one) so a module added
/// in a future version is born enabled for existing users.
function load(): WorkbenchGroupKey[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    // Drop unknown keys (renamed/removed groups) and locked ones (a stale
    // entry must never switch off Trust).
    return parsed.filter(
      (k): k is WorkbenchGroupKey =>
        typeof k === "string" && MODULES.some((m) => m.key === k && !m.locked),
    );
  } catch {
    return [];
  }
}

function persist(key: string, value: WorkbenchGroupKey[]) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* ignore */
  }
}

interface ModulesState {
  disabled: WorkbenchGroupKey[];
  setEnabled: (key: WorkbenchGroupKey, enabled: boolean) => void;
  isEnabled: (key: WorkbenchGroupKey) => boolean;
}

export const useModulesStore = create<ModulesState>((set, get) => ({
  disabled: effective(load(), loadOptIn()),
  setEnabled: (key, enabled) => {
    const mod = MODULES.find((m) => m.key === key);
    if (!mod || mod.locked) return;
    const disabled = get().disabled.filter((k) => k !== key);
    if (!enabled) disabled.push(key);
    if (mod.experimental) {
      const optIn = loadOptIn().filter((k) => k !== key);
      if (enabled) optIn.push(key);
      persist(OPT_IN_KEY, optIn);
    } else {
      persist(
        STORAGE_KEY,
        disabled.filter((k) => !isExperimental(k)),
      );
    }
    set({ disabled });
  },
  isEnabled: (key) => !get().disabled.includes(key),
}));

/// Whether a tab is reachable right now. Groupless tabs (transfer, feedback)
/// belong to no module and are always on.
export function isTabEnabled(tab: WorkbenchTab): boolean {
  const key = moduleForTab(tab);
  return !key || !useModulesStore.getState().disabled.includes(key);
}

/// Fallback target when the active tab's module gets switched off: the first
/// tab of the first enabled group, in strip order. Always resolves — Trust is
/// locked-on, so at least one group is enabled.
export function firstEnabledTab(): WorkbenchTab {
  const disabled = useModulesStore.getState().disabled;
  const group = WORKBENCH_GROUPS.find((g) => !disabled.includes(g.key));
  return group ? group.tabs[0] : "permissions";
}
