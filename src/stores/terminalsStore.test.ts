import { beforeEach, describe, expect, it, vi } from "vitest";

import type { PersistedSession } from "../types/domain";

// The store hits the backend and the toast store — isolate both. The toast spy
// matters: issue #72's second round was exactly "saving broke and nothing said
// so"; these tests pin the signals.
const world = vi.hoisted(() => ({
  saved: [] as { root: string; payload: PersistedSession[] }[],
  listResult: [] as PersistedSession[],
  listError: null as string | null,
  saveError: null as string | null,
  toasts: [] as { msg: string; kind: string }[],
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    sessionsList: async (root: string) => {
      if (world.listError) throw new Error(world.listError);
      void root;
      return world.listResult;
    },
    sessionsSave: async (root: string, payload: PersistedSession[]) => {
      if (world.saveError) throw new Error(world.saveError);
      world.saved.push({ root, payload });
    },
  },
}));
vi.mock("./toastStore", () => ({
  useToastStore: {
    getState: () => ({
      show: (msg: string, kind: string) => {
        world.toasts.push({ msg, kind });
      },
    }),
  },
}));

import { useTerminalsStore } from "./terminalsStore";

function persisted(id: string, over: Partial<PersistedSession> = {}): PersistedSession {
  return {
    id,
    name: `sess-${id}`,
    cwd: "/repo",
    createdAtMs: 1,
    scrollback: "old output",
    ...over,
  };
}

async function resetToReady() {
  world.listResult = [];
  world.listError = null;
  world.saveError = null;
  await useTerminalsStore.getState().hydrate("/repo");
  world.saved = [];
  world.toasts = [];
  // Drain the module-level failure counter with one successful persist.
  useTerminalsStore.getState().add("/repo");
  await useTerminalsStore.getState().persist();
  world.saved = [];
  useTerminalsStore.setState({ sessions: [], activeId: null });
}

beforeEach(async () => {
  await resetToReady();
});

describe("hydrate (the issue #72 contract)", () => {
  it("maps persisted sessions to stopped ones and opens the ready gate", async () => {
    world.listResult = [persisted("a"), persisted("b", { claudeSessionId: "c-9" })];
    await useTerminalsStore.getState().hydrate("/repo");
    const s = useTerminalsStore.getState();
    expect(s.ready).toBe(true);
    expect(s.sessions.map((x) => x.status)).toEqual(["stopped", "stopped"]);
    expect(s.sessions[0].initialScrollback).toBe("old output");
    expect(s.sessions[1].claudeSessionId).toBe("c-9");
  });

  it("read failure keeps the gate closed AND tells the user saving is off", async () => {
    world.listError = "corrupt json";
    await useTerminalsStore.getState().hydrate("/repo");
    expect(useTerminalsStore.getState().ready).toBe(false);
    expect(world.toasts.some((t) => t.kind === "error" && t.msg.includes("NOT being saved"))).toBe(
      true,
    );
  });

  it("persist is a hard no-op while the gate is closed — a failed read can never overwrite history", async () => {
    world.listError = "corrupt json";
    await useTerminalsStore.getState().hydrate("/repo");
    useTerminalsStore.getState().add("/repo");
    await useTerminalsStore.getState().persist();
    expect(world.saved).toEqual([]);
  });
});

describe("persist payload", () => {
  it("live sessions save their live scrollback; stopped ones keep the old one", async () => {
    world.listResult = [persisted("old")];
    await useTerminalsStore.getState().hydrate("/repo");
    world.saved = [];

    const liveId = useTerminalsStore.getState().add("/repo", "worker");
    useTerminalsStore.getState().appendOutput(liveId, "fresh bytes");
    await useTerminalsStore.getState().persist();

    const payload = world.saved[world.saved.length - 1].payload;
    const oldOne = payload.find((p) => p.id === "old")!;
    const liveOne = payload.find((p) => p.id === liveId)!;
    expect(oldOne.scrollback).toBe("old output");
    expect(liveOne.scrollback).toBe("fresh bytes");
  });

  it("scrollback is capped, keeping the tail", async () => {
    const id = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().appendOutput(id, "x".repeat(60_000));
    useTerminalsStore.getState().appendOutput(id, "END");
    await useTerminalsStore.getState().persist();
    const sb = world.saved[world.saved.length - 1].payload[0].scrollback;
    expect(sb.length).toBeLessThanOrEqual(50_000);
    expect(sb.endsWith("END")).toBe(true);
  });

  it("save failures toast on the first and every 10th — never silently", async () => {
    useTerminalsStore.getState().add("/repo");
    world.saveError = "disk full";
    for (let i = 1; i <= 10; i++) {
      await useTerminalsStore.getState().persist();
    }
    const errors = world.toasts.filter((t) => t.kind === "error");
    expect(errors.length).toBe(2); // 1st and 10th
    expect(errors[0].msg).toContain("disk full");

    // Recovery resets the counter: next failure toasts again as "first".
    world.saveError = null;
    await useTerminalsStore.getState().persist();
    world.toasts = [];
    world.saveError = "disk full again";
    await useTerminalsStore.getState().persist();
    expect(world.toasts.filter((t) => t.kind === "error").length).toBe(1);
    world.saveError = null;
    await useTerminalsStore.getState().persist();
  });
});

describe("session lifecycle", () => {
  it("add picks the first free 'shell N' name and activates the session", () => {
    const a = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().rename(a, "shell 3");
    const c = useTerminalsStore.getState().add("/repo");
    const names = useTerminalsStore.getState().sessions.map((s) => s.name);
    expect(names).toContain("shell 1");
    expect(names).toContain("shell 2");
    // "shell 1" freed by the rename of a → reused before "shell 4".
    expect(useTerminalsStore.getState().sessions.find((s) => s.id === c)!.name).toBe("shell 1");
    expect(useTerminalsStore.getState().activeId).toBe(c);
  });

  it("close removes the session, hands focus to the last remaining, and persists", async () => {
    const a = useTerminalsStore.getState().add("/repo");
    const b = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().setActive(b);
    await useTerminalsStore.getState().close(b);
    const s = useTerminalsStore.getState();
    expect(s.sessions.map((x) => x.id)).toEqual([a]);
    expect(s.activeId).toBe(a);
    expect(world.saved.length).toBe(1);
  });

  it("resume flips a stopped session to live and focuses it", async () => {
    world.listResult = [persisted("a")];
    await useTerminalsStore.getState().hydrate("/repo");
    useTerminalsStore.getState().resume("a");
    const s = useTerminalsStore.getState();
    expect(s.sessions[0].status).toBe("live");
    expect(s.activeId).toBe("a");
  });
});

describe("silent auto-naming (only ever replaces a default 'shell N')", () => {
  it("renames a default-named session once, silently", () => {
    const id = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().autoName(id, "Fix login bug");
    expect(useTerminalsStore.getState().sessions[0].name).toBe("Fix login bug");
    // One shot: a later (different) first-prompt heuristic can't rename again.
    useTerminalsStore.getState().autoName(id, "Something else");
    expect(useTerminalsStore.getState().sessions[0].name).toBe("Fix login bug");
  });

  it("never touches a user-chosen or ticket name", () => {
    const id = useTerminalsStore.getState().add("/repo", "FIX-123");
    useTerminalsStore.getState().autoName(id, "Renamed by robot");
    expect(useTerminalsStore.getState().sessions[0].name).toBe("FIX-123");
  });

  it("manual rename closes the auto-name window for good", () => {
    const id = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().rename(id, "shell 1");
    // Still shaped like a default name, but the USER typed it — hands off.
    useTerminalsStore.getState().autoName(id, "Robot name");
    expect(useTerminalsStore.getState().sessions[0].name).toBe("shell 1");
  });
});

describe("archive / history", () => {
  it("archive hides a stopped session and persists; resume brings it back", async () => {
    world.listResult = [persisted("a")];
    await useTerminalsStore.getState().hydrate("/repo");
    world.saved = [];

    useTerminalsStore.getState().archive("a");
    let s = useTerminalsStore.getState().sessions[0];
    expect(s.archived).toBe(true);
    await new Promise((r) => setTimeout(r, 0));
    expect(world.saved.length).toBe(1);
    expect(world.saved[0].payload[0].archived).toBe(true);

    useTerminalsStore.getState().resume("a");
    s = useTerminalsStore.getState().sessions[0];
    expect(s.archived).toBe(false);
    expect(s.status).toBe("live");
  });

  it("live sessions cannot be archived (close them first)", () => {
    const id = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().archive(id);
    expect(useTerminalsStore.getState().sessions[0].archived).toBeUndefined();
  });

  it("hydrate auto-archives sessions stale for over 7 days, keeps recent ones", async () => {
    const DAY = 86_400_000;
    world.listResult = [
      persisted("old", { lastActiveMs: Date.now() - 8 * DAY }),
      persisted("fresh", { lastActiveMs: Date.now() - 2 * DAY }),
      // Pre-lastActiveMs sessions fall back to createdAtMs (ancient here).
      persisted("ancient"),
    ];
    await useTerminalsStore.getState().hydrate("/repo");
    const byId = new Map(useTerminalsStore.getState().sessions.map((s) => [s.id, s]));
    expect(byId.get("old")!.archived).toBe(true);
    expect(byId.get("fresh")!.archived).toBe(false);
    expect(byId.get("ancient")!.archived).toBe(true);
  });

  it("persist stamps live sessions' lastActiveMs at save time", async () => {
    const id = useTerminalsStore.getState().add("/repo");
    const before = Date.now();
    await useTerminalsStore.getState().persist();
    const saved = world.saved[world.saved.length - 1].payload.find((p) => p.id === id)!;
    expect(saved.lastActiveMs).toBeGreaterThanOrEqual(before);
  });
});

describe("claude session binding", () => {
  it("setClaudeSessionId persists immediately and skips no-op rebinds", async () => {
    const id = useTerminalsStore.getState().add("/repo");
    useTerminalsStore.getState().setClaudeSessionId(id, "claude-1");
    // persist() is fire-and-forget inside the setter — let it settle.
    await new Promise((r) => setTimeout(r, 0));
    expect(world.saved.length).toBe(1);
    expect(world.saved[0].payload[0].claudeSessionId).toBe("claude-1");

    useTerminalsStore.getState().setClaudeSessionId(id, "claude-1");
    await new Promise((r) => setTimeout(r, 0));
    expect(world.saved.length).toBe(1);
  });
});

describe("login-only sessions", () => {
  it("add carries the transient loginOnly flag and persist never saves it", async () => {
    const id = useTerminalsStore
      .getState()
      .add("/repo", "claude login", undefined, "claude", undefined, undefined, undefined, true);
    expect(useTerminalsStore.getState().sessions.find((s) => s.id === id)!.loginOnly).toBe(true);
    await useTerminalsStore.getState().persist();
    const saved = world.saved[world.saved.length - 1].payload[0] as unknown as Record<
      string,
      unknown
    >;
    expect("loginOnly" in saved).toBe(false);
  });
});
