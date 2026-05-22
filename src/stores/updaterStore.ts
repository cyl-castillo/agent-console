import { create } from "zustand";
import { checkForUpdate, installAndRelaunch, type UpdateInfo } from "../ipc/updater";

type Phase = "idle" | "checking" | "available" | "installing" | "error" | "uptodate";

interface UpdaterState {
  phase: Phase;
  info: UpdateInfo | null;
  error: string | null;
  check: (opts?: { silentIfNone?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

export const useUpdaterStore = create<UpdaterState>((set, get) => ({
  phase: "idle",
  info: null,
  error: null,

  async check(opts) {
    if (get().phase === "checking" || get().phase === "installing") return;
    set({ phase: "checking", error: null });
    try {
      const info = await checkForUpdate();
      if (info) {
        set({ phase: "available", info });
      } else {
        set({ phase: opts?.silentIfNone ? "idle" : "uptodate", info: null });
      }
    } catch (e) {
      set({ phase: "error", error: e instanceof Error ? e.message : String(e) });
    }
  },

  async install() {
    const { info } = get();
    if (!info) return;
    set({ phase: "installing", error: null });
    try {
      await installAndRelaunch(info);
    } catch (e) {
      set({ phase: "error", error: e instanceof Error ? e.message : String(e) });
    }
  },

  dismiss() {
    set({ phase: "idle", info: null, error: null });
  },
}));
