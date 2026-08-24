import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mutable world the mocked stores read from. vi.mock factories are hoisted,
// so the shared state must be hoisted too.
const world = vi.hoisted(() => ({
  sessions: [] as { id: string; status: string }[],
  activeId: null as string | null,
  tabs: [] as string[],
  toasts: [] as string[],
}));

vi.mock("../stores/terminalsStore", () => ({
  useTerminalsStore: {
    getState: () => ({ sessions: world.sessions, activeId: world.activeId }),
  },
}));
vi.mock("../stores/uiStore", () => ({
  useUIStore: {
    getState: () => ({ setTab: (t: string) => world.tabs.push(t) }),
  },
}));
vi.mock("../stores/toastStore", () => ({
  useToastStore: {
    getState: () => ({ show: (msg: string) => world.toasts.push(msg) }),
  },
}));

import { typeIntoActiveSession } from "./termInput";

// Node test env has no DOM: stub the two globals the module touches at call
// time. Events dispatched land in `dispatched`; clipboard writes in `clips`.
const dispatched: { type: string; detail: unknown }[] = [];
const clips: string[] = [];

beforeEach(() => {
  world.sessions = [];
  world.activeId = null;
  world.tabs = [];
  world.toasts = [];
  dispatched.length = 0;
  clips.length = 0;
  vi.stubGlobal("window", {
    dispatchEvent: (e: { type: string; detail: unknown }) => {
      dispatched.push({ type: e.type, detail: e.detail });
      return true;
    },
  });
  vi.stubGlobal(
    "CustomEvent",
    class {
      type: string;
      detail: unknown;
      constructor(type: string, init?: { detail?: unknown }) {
        this.type = type;
        this.detail = init?.detail;
      }
    },
  );
  vi.stubGlobal("navigator", {
    clipboard: { writeText: async (t: string) => void clips.push(t) },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("typeIntoActiveSession", () => {
  it("dispatches ac:term-input to the live active session, trimming trailing whitespace", async () => {
    world.sessions = [{ id: "t-1", status: "live" }];
    world.activeId = "t-1";
    const ok = await typeIntoActiveSession("hello world  \n");
    expect(ok).toBe(true);
    expect(dispatched).toEqual([
      { type: "ac:term-input", detail: { sessionId: "t-1", data: "hello world" } },
    ]);
    expect(world.tabs).toEqual(["terminal"]);
  });

  it("appends Enter when submit is requested", async () => {
    world.sessions = [{ id: "t-1", status: "live" }];
    world.activeId = "t-1";
    await typeIntoActiveSession("run it", { submit: true });
    expect(dispatched[0].detail).toEqual({ sessionId: "t-1", data: "run it\r" });
  });

  it("falls back to the clipboard when there is no active session", async () => {
    const ok = await typeIntoActiveSession("orphan text");
    expect(ok).toBe(false);
    expect(dispatched).toEqual([]);
    expect(clips).toEqual(["orphan text"]);
    expect(world.toasts).toHaveLength(1);
  });

  it("falls back to the clipboard when the active session is stopped", async () => {
    world.sessions = [{ id: "t-1", status: "stopped" }];
    world.activeId = "t-1";
    const ok = await typeIntoActiveSession("orphan text");
    expect(ok).toBe(false);
    expect(dispatched).toEqual([]);
    expect(clips).toEqual(["orphan text"]);
  });

  it("ignores empty input", async () => {
    world.sessions = [{ id: "t-1", status: "live" }];
    world.activeId = "t-1";
    const ok = await typeIntoActiveSession("   \n");
    expect(ok).toBe(false);
    expect(dispatched).toEqual([]);
  });
});
