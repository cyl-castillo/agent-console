import { beforeEach, describe, expect, it, vi } from "vitest";

// Mutable world the mocked sibling stores read from. vi.mock factories are
// hoisted, so the shared state must be hoisted too.
const world = vi.hoisted(() => ({
  projectRoot: "/repo" as string | null,
  sessions: [] as { id: string; name: string; cwd: string; status: string }[],
  activeId: null as string | null,
  branches: [] as { name: string }[],
  branch: "main" as string | null,
  dirty: false,
  checkedOut: [] as string[],
  recent: [] as { prompt?: string; snapshotCommitSha?: string }[],
  indexedFiles: [] as string[],
  indexCalls: 0,
  indexError: null as string | null,
  logins: [] as string[],
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    paletteIndexFiles: async () => {
      world.indexCalls++;
      if (world.indexError) throw new Error(world.indexError);
      return world.indexedFiles;
    },
  },
}));
vi.mock("./sessionStore", () => ({
  useSessionStore: {
    getState: () => ({ project: world.projectRoot ? { root: world.projectRoot } : null }),
  },
}));
vi.mock("./terminalsStore", () => ({
  useTerminalsStore: {
    getState: () => ({
      sessions: world.sessions,
      activeId: world.activeId,
      setActive: () => {},
      close: async () => {},
    }),
  },
}));
vi.mock("./changesStore", () => ({
  useChangesStore: {
    getState: () => ({
      branches: world.branches,
      branchesLoading: false,
      status: world.branch
        ? { branch: world.branch, changes: world.dirty ? [{ path: "x" }] : [] }
        : null,
      loadBranches: async () => {},
      refresh: async () => {},
      checkoutBranch: async (name: string) => {
        world.checkedOut.push(name);
      },
    }),
  },
}));
vi.mock("./skillsStore", () => ({
  useSkillsStore: {
    getState: () => ({
      recent: world.recent,
      undoRestoreSha: null,
      restoreSnapshot: async () => {},
    }),
  },
}));
vi.mock("./previewStore", () => ({
  usePreviewStore: { getState: () => ({ open: async () => {} }) },
}));
vi.mock("./themeStore", () => ({
  useThemeStore: { getState: () => ({ toggle: () => {} }) },
}));
vi.mock("./toastStore", () => ({
  useToastStore: { getState: () => ({ show: () => {} }) },
}));
vi.mock("./updaterStore", () => ({
  useUpdaterStore: { getState: () => ({ check: async () => {}, phase: "idle" }) },
}));
vi.mock("../lib/reportProblem", () => ({ reportProblem: () => {} }));
vi.mock("../lib/loginSession", () => ({
  startLoginSession: (agent: string) => {
    world.logins.push(agent);
  },
}));
vi.mock("../lib/termInput", () => ({ typeIntoActiveSession: async () => {} }));

import { usePaletteStore } from "./paletteStore";

function results(query: string) {
  usePaletteStore.setState({ query });
  return usePaletteStore.getState().results();
}

beforeEach(() => {
  world.projectRoot = "/repo";
  world.sessions = [];
  world.activeId = null;
  world.branches = [];
  world.branch = "main";
  world.dirty = false;
  world.checkedOut = [];
  world.recent = [];
  world.indexedFiles = [];
  world.indexCalls = 0;
  world.indexError = null;
  world.logins = [];
  usePaletteStore.setState({
    open: false,
    query: "",
    selectedIndex: 0,
    files: [],
    filesProjectRoot: null,
    filesLoading: false,
    filesError: null,
    pendingBranchSwitch: null,
  });
  vi.stubGlobal("window", { dispatchEvent: vi.fn() });
});

describe("palette results — modes and filtering", () => {
  it("empty query surfaces actions (and sessions, when any exist)", () => {
    world.sessions = [{ id: "s1", name: "backend", cwd: "/repo", status: "live" }];
    const items = results("");
    expect(items.some((i) => i.kind === "action" && i.label === "Open Terminal")).toBe(true);
    expect(items.some((i) => i.kind === "session" && i.label === "backend")).toBe(true);
  });

  it("'>' restricts to actions; keywords match ('testigo' finds Proof)", () => {
    world.sessions = [{ id: "s1", name: "backend", cwd: "/repo", status: "live" }];
    const items = results(">testigo");
    expect(items.length).toBeGreaterThan(0);
    expect(items.every((i) => i.kind === "action")).toBe(true);
    expect(items[0].label).toBe("Open Proof");
  });

  it("every workbench tab that left the strip is still reachable (transfer, feedback)", () => {
    const labels = results(">").map((i) => i.label);
    expect(labels).toContain("Open Transfer");
    expect(labels).toContain("Open Feedback");
  });

  it("':' restricts to sessions and fuzzy-matches on name", () => {
    world.sessions = [
      { id: "s1", name: "backend", cwd: "/repo", status: "live" },
      { id: "s2", name: "frontend", cwd: "/repo", status: "live" },
    ];
    const items = results(":front");
    expect(items.map((i) => i.kind)).toEqual(["session"]);
    expect(items[0].label).toBe("frontend");
  });

  it("actions gated by available() disappear when unavailable", () => {
    world.activeId = null;
    expect(results(">close active").some((i) => i.label === "Close Active Session")).toBe(false);
    world.sessions = [{ id: "s1", name: "backend", cwd: "/repo", status: "live" }];
    world.activeId = "s1";
    expect(results(">close active").some((i) => i.label === "Close Active Session")).toBe(true);
  });

  it("files rank basename matches first and carry the dir as hint", async () => {
    world.indexedFiles = ["src/components/App.tsx", "docs/apparatus.md"];
    await usePaletteStore.getState().reloadIndex();
    const files = results("app").filter((i) => i.kind === "file");
    expect(files[0].label).toBe("App.tsx");
    expect(files[0].hint).toBe("src/components");
  });
});

describe("palette file index lifecycle", () => {
  it("ensureIndex loads once per project and then caches", async () => {
    world.indexedFiles = ["a.ts"];
    await usePaletteStore.getState().ensureIndex("/repo");
    await usePaletteStore.getState().ensureIndex("/repo");
    expect(world.indexCalls).toBe(1);
  });

  it("index failure is surfaced, not swallowed", async () => {
    world.indexError = "walk failed";
    await usePaletteStore.getState().reloadIndex();
    const s = usePaletteStore.getState();
    expect(s.filesLoading).toBe(false);
    expect(s.filesError).toContain("walk failed");
  });

  it("resetForProject clears the stale index of the previous project", async () => {
    world.indexedFiles = ["old.ts"];
    await usePaletteStore.getState().reloadIndex();
    usePaletteStore.getState().resetForProject("/other");
    const s = usePaletteStore.getState();
    expect(s.files).toEqual([]);
    expect(s.filesProjectRoot).toBe("/other");
  });
});

describe("branch switching — the double-Enter guard", () => {
  beforeEach(() => {
    world.branches = [{ name: "main" }, { name: "feat/x" }];
    world.branch = "main";
  });

  it("clean tree: switching is a single step", async () => {
    const item = results("@feat").find((i) => i.kind === "branch" && i.label === "feat/x")!;
    expect(item.warn).toBeUndefined();
    await usePaletteStore.getState().execute(item);
    expect(world.checkedOut).toEqual(["feat/x"]);
  });

  it("dirty tree: first Enter arms the confirm, second Enter switches", async () => {
    world.dirty = true;
    usePaletteStore.setState({ open: true });
    const item = results("@feat").find((i) => i.kind === "branch" && i.label === "feat/x")!;
    expect(item.warn).toBeTruthy();

    await usePaletteStore.getState().execute(item);
    expect(world.checkedOut).toEqual([]);
    expect(usePaletteStore.getState().pendingBranchSwitch).toBe(item.id);
    expect(usePaletteStore.getState().open).toBe(true);

    await usePaletteStore.getState().execute(item);
    expect(world.checkedOut).toEqual(["feat/x"]);
    expect(usePaletteStore.getState().open).toBe(false);
  });

  it("typing again disarms a pending branch confirm", () => {
    world.dirty = true;
    usePaletteStore.setState({ pendingBranchSwitch: "branch:feat/x" });
    usePaletteStore.getState().setQuery("@fea");
    expect(usePaletteStore.getState().pendingBranchSwitch).toBeNull();
  });

  it("the current branch never offers a confirm and sinks in the ranking", () => {
    world.dirty = true;
    const items = results("@");
    const current = items.find((i) => i.label === "main")!;
    expect(current.hint).toBe("current");
    expect(current.warn).toBeUndefined();
  });
});

describe("execute", () => {
  it("runs the item and always closes the palette afterwards", async () => {
    usePaletteStore.setState({ open: true });
    let ran = false;
    await usePaletteStore.getState().execute({
      id: "action:test",
      kind: "action",
      label: "t",
      score: 1,
      run: () => {
        ran = true;
      },
    });
    expect(ran).toBe(true);
    expect(usePaletteStore.getState().open).toBe(false);
  });

  it("closes even when the action throws", async () => {
    usePaletteStore.setState({ open: true });
    await expect(
      usePaletteStore.getState().execute({
        id: "action:boom",
        kind: "action",
        label: "b",
        score: 1,
        run: () => {
          throw new Error("boom");
        },
      }),
    ).rejects.toThrow("boom");
    expect(usePaletteStore.getState().open).toBe(false);
  });
});

describe("engine login repair actions", () => {
  it("'>fix login' surfaces both engines when a project is open", () => {
    const labels = results(">fix login").map((i) => i.label);
    expect(labels).toContain("Fix Claude login");
    expect(labels).toContain("Fix Codex login");
  });

  it("hidden without a project (a login session needs a cwd)", () => {
    world.projectRoot = null;
    const labels = results(">fix login").map((i) => i.label);
    expect(labels).not.toContain("Fix Claude login");
  });

  it("executing the action starts the login session for the right engine", async () => {
    const item = results(">fix codex").find((i) => i.label === "Fix Codex login")!;
    await usePaletteStore.getState().execute(item);
    expect(world.logins).toEqual(["codex"]);
  });
});
