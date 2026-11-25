import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { AgentKind } from "../agents/profiles";
import type { Preflight, PreflightTool } from "../types/domain";

interface PreflightState {
  result: Preflight | null;
  checking: boolean;
  /// Run the environment check. Re-runnable (the "Re-check" button) but
  /// coalesces concurrent calls so mount-from-two-places probes once.
  check: () => Promise<void>;
}

export const usePreflightStore = create<PreflightState>((set, get) => ({
  result: null,
  checking: false,

  check: async () => {
    if (get().checking) return;
    set({ checking: true });
    try {
      const result = await ipc.preflightCheck();
      set({ result, checking: false });
    } catch {
      // A failed probe shouldn't blank the last good result.
      set({ checking: false });
    }
  },
}));

/// The agent CLIs the console can drive. One of them is enough to work.
export const ENGINE_TOOLS = ["claude", "codex", "opencode"] as const;

/// The engines whose CLI the preflight found, in ENGINE_TOOLS order.
export function enginesFound(result: Preflight | null): string[] {
  return ENGINE_TOOLS.filter((name) => toolStatus(result, name)?.found);
}

/// The engine to start with when nobody chose one: the first one found, in
/// ENGINE_TOOLS order, so Claude whenever it is installed. Undefined before
/// the probe answers and when no agent CLI is installed.
export function firstEngineFound(result: Preflight | null): AgentKind | undefined {
  return ENGINE_TOOLS.find((name) => toolStatus(result, name)?.found);
}

/// The result of the check that is running, once it lands; what there is
/// (possibly nothing) when none is running or it takes longer than `ms`.
export function preflightSettled(ms = 3000): Promise<Preflight | null> {
  const now = usePreflightStore.getState();
  if (now.result || !now.checking) return Promise.resolve(now.result);
  return new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      unsubscribe();
      resolve(usePreflightStore.getState().result);
    };
    const timer = setTimeout(finish, ms);
    const unsubscribe = usePreflightStore.subscribe((s) => {
      if (!s.checking) finish();
    });
  });
}

/// Look up one tool's status from a preflight result.
export function toolStatus(result: Preflight | null, name: string): PreflightTool | undefined {
  return result?.tools.find((t) => t.name === name);
}
