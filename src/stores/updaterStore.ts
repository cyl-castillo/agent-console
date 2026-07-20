import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import { checkForUpdate, installAndRelaunch, type UpdateInfo } from "../ipc/updater";
import { checkGithubRelease, type ManualUpdateInfo } from "../ipc/githubRelease";
import { openUrl } from "@tauri-apps/plugin-opener";

type Phase =
  | "idle"
  | "checking"
  | "available"
  | "available-manual"
  | "installing"
  | "error"
  | "uptodate"
  /// Running inside a snap: snapd auto-refreshes the app, so the in-app
  /// updater stands down entirely (two updaters would fight over the install).
  | "snap-managed";

async function isSnapManaged(): Promise<boolean> {
  try {
    return (await ipc.appBuildInfo()).snap;
  } catch {
    return false;
  }
}

interface UpdaterState {
  phase: Phase;
  info: UpdateInfo | null;
  manualInfo: ManualUpdateInfo | null;
  error: string | null;
  check: (opts?: { silentIfNone?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  openDownload: () => Promise<void>;
  dismiss: () => void;
}

export const useUpdaterStore = create<UpdaterState>((set, get) => ({
  phase: "idle",
  info: null,
  manualInfo: null,
  error: null,

  async check(opts) {
    if (get().phase === "checking" || get().phase === "installing") return;
    if (await isSnapManaged()) {
      set({
        phase: opts?.silentIfNone ? "idle" : "snap-managed",
        info: null,
        manualInfo: null,
      });
      return;
    }
    set({ phase: "checking", error: null });
    try {
      const info = await checkForUpdate();
      if (info) {
        set({ phase: "available", info, manualInfo: null });
        return;
      }
      // Tauri updater found nothing — either truly up-to-date, or running from
      // a package format the plugin can't update (e.g. .deb/.rpm on Linux).
      // Fall back to the GitHub Releases API so we can at least notify.
      try {
        const manual = await checkGithubRelease();
        if (manual) {
          set({ phase: "available-manual", manualInfo: manual, info: null });
          return;
        }
      } catch {
        // Ignore fallback errors; treat as up-to-date for UX purposes.
      }
      set({
        phase: opts?.silentIfNone ? "idle" : "uptodate",
        info: null,
        manualInfo: null,
      });
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

  async openDownload() {
    const { manualInfo } = get();
    if (!manualInfo) return;
    try {
      await openUrl(manualInfo.url);
    } catch (e) {
      set({ phase: "error", error: e instanceof Error ? e.message : String(e) });
    }
  },

  dismiss() {
    set({ phase: "idle", info: null, manualInfo: null, error: null });
  },
}));
