import { beforeEach, describe, expect, it, vi } from "vitest";

const world = vi.hoisted(() => ({
  tauriUpdate: null as { version: string } | null,
  tauriError: null as string | null,
  manualUpdate: null as { version: string; url: string } | null,
  manualError: null as string | null,
  installed: 0,
  installError: null as string | null,
  opened: [] as string[],
  snap: false,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    appBuildInfo: async () => ({ commit: "test", buildTimeSecs: 0, debug: true, snap: world.snap }),
  },
}));

vi.mock("../ipc/updater", () => ({
  checkForUpdate: async () => {
    if (world.tauriError) throw new Error(world.tauriError);
    return world.tauriUpdate;
  },
  installAndRelaunch: async () => {
    if (world.installError) throw new Error(world.installError);
    world.installed++;
  },
}));
vi.mock("../ipc/githubRelease", () => ({
  checkGithubRelease: async () => {
    if (world.manualError) throw new Error(world.manualError);
    return world.manualUpdate;
  },
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: async (url: string) => {
    world.opened.push(url);
  },
}));

import { useUpdaterStore } from "./updaterStore";

beforeEach(() => {
  world.tauriUpdate = null;
  world.tauriError = null;
  world.manualUpdate = null;
  world.manualError = null;
  world.installed = 0;
  world.installError = null;
  world.opened = [];
  world.snap = false;
  useUpdaterStore.setState({ phase: "idle", info: null, manualInfo: null, error: null });
});

describe("snap-managed installs", () => {
  it("the in-app updater stands down inside a snap (snapd owns updates)", async () => {
    world.snap = true;
    world.tauriUpdate = { version: "9.9.9" }; // must never be consulted
    await useUpdaterStore.getState().check();
    const s = useUpdaterStore.getState();
    expect(s.phase).toBe("snap-managed");
    expect(s.info).toBeNull();

    // Startup (silent) check: quietly idle, no banner state at all.
    await useUpdaterStore.getState().check({ silentIfNone: true });
    expect(useUpdaterStore.getState().phase).toBe("idle");
  });
});

describe("updater check", () => {
  it("a Tauri-updatable release lands in 'available'", async () => {
    world.tauriUpdate = { version: "9.9.9" };
    await useUpdaterStore.getState().check();
    const s = useUpdaterStore.getState();
    expect(s.phase).toBe("available");
    expect(s.info?.version).toBe("9.9.9");
    expect(s.manualInfo).toBeNull();
  });

  it("falls back to GitHub for package formats the plugin can't update (.deb/.rpm)", async () => {
    world.manualUpdate = { version: "9.9.9", url: "https://example/releases" };
    await useUpdaterStore.getState().check();
    const s = useUpdaterStore.getState();
    expect(s.phase).toBe("available-manual");
    expect(s.manualInfo?.url).toBe("https://example/releases");
    expect(s.info).toBeNull();
  });

  it("nothing anywhere → 'uptodate', or silent idle when asked (startup check)", async () => {
    await useUpdaterStore.getState().check();
    expect(useUpdaterStore.getState().phase).toBe("uptodate");

    await useUpdaterStore.getState().check({ silentIfNone: true });
    expect(useUpdaterStore.getState().phase).toBe("idle");
  });

  it("a broken GitHub fallback is not an error — it reads as up to date", async () => {
    world.manualError = "rate limited";
    await useUpdaterStore.getState().check();
    expect(useUpdaterStore.getState().phase).toBe("uptodate");
  });

  it("a broken primary check IS surfaced", async () => {
    world.tauriError = "network down";
    await useUpdaterStore.getState().check();
    const s = useUpdaterStore.getState();
    expect(s.phase).toBe("error");
    expect(s.error).toContain("network down");
  });

  it("won't re-enter while installing", async () => {
    useUpdaterStore.setState({ phase: "installing" });
    world.tauriUpdate = { version: "9.9.9" };
    await useUpdaterStore.getState().check();
    expect(useUpdaterStore.getState().phase).toBe("installing");
  });
});

describe("install / download / dismiss", () => {
  it("install without an update available is a no-op", async () => {
    await useUpdaterStore.getState().install();
    expect(world.installed).toBe(0);
    expect(useUpdaterStore.getState().phase).toBe("idle");
  });

  it("a failed install surfaces the error instead of hanging in 'installing'", async () => {
    world.tauriUpdate = { version: "9.9.9" };
    await useUpdaterStore.getState().check();
    world.installError = "signature mismatch";
    await useUpdaterStore.getState().install();
    const s = useUpdaterStore.getState();
    expect(s.phase).toBe("error");
    expect(s.error).toContain("signature mismatch");
  });

  it("openDownload opens the manual release URL", async () => {
    world.manualUpdate = { version: "9.9.9", url: "https://example/dl" };
    await useUpdaterStore.getState().check();
    await useUpdaterStore.getState().openDownload();
    expect(world.opened).toEqual(["https://example/dl"]);
  });

  it("dismiss clears everything back to idle", async () => {
    world.tauriUpdate = { version: "9.9.9" };
    await useUpdaterStore.getState().check();
    useUpdaterStore.getState().dismiss();
    const s = useUpdaterStore.getState();
    expect(s.phase).toBe("idle");
    expect(s.info).toBeNull();
    expect(s.error).toBeNull();
  });
});
