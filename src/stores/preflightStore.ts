import { create } from "zustand";

import { ipc } from "../ipc/tauri";
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

/// Look up one tool's status from a preflight result.
export function toolStatus(result: Preflight | null, name: string): PreflightTool | undefined {
  return result?.tools.find((t) => t.name === name);
}
