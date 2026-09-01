import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSessionBinding, PersistedSession } from "../types/domain";

const world = vi.hoisted(() => ({
  bindings: [] as AgentSessionBinding[],
  calls: 0,
  fail: false,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    termAgentSessions: async () => {
      world.calls += 1;
      if (world.fail) throw new Error("no backend");
      return world.bindings;
    },
    sessionsList: async () => [] as PersistedSession[],
    sessionsSave: async () => {},
  },
}));
vi.mock("../stores/toastStore", () => ({
  useToastStore: { getState: () => ({ show: () => {} }) },
}));

import { useTerminalsStore } from "../stores/terminalsStore";
import { adoptLiveResumeHandles } from "./resumeHandles";

type SessionSeed = {
  id: string;
  agent?: "claude" | "codex";
  agentSessionId?: string;
  status?: "live" | "stopped";
};

function seed(sessions: SessionSeed[]) {
  useTerminalsStore.setState({
    sessions: sessions.map((s) => ({
      id: s.id,
      name: s.id,
      cwd: "/w",
      createdAtMs: 0,
      initialScrollback: "",
      liveScrollback: "",
      status: s.status ?? "live",
      agent: s.agent,
      agentSessionId: s.agentSessionId,
    })),
    activeId: sessions[0]?.id ?? null,
  });
}

const idOf = (id: string) =>
  useTerminalsStore.getState().sessions.find((s) => s.id === id)?.agentSessionId;

describe("adoptLiveResumeHandles", () => {
  beforeEach(() => {
    world.bindings = [];
    world.calls = 0;
    world.fail = false;
  });

  it("gives an unhooked terminal the id of the agent actually running in it", async () => {
    seed([{ id: "t1" }]);
    world.bindings = [{ termKey: "t1", sessionId: "sess-1" }];
    expect(await adoptLiveResumeHandles()).toBe(1);
    expect(idOf("t1")).toBe("sess-1");
  });

  it("does not ask the backend when every terminal already has an id", async () => {
    // The steady state: the hook works, so the timer must cost nothing.
    seed([{ id: "t1", agentSessionId: "from-hook" }]);
    expect(await adoptLiveResumeHandles()).toBe(0);
    expect(world.calls).toBe(0);
  });

  it("never overwrites an id the hook already captured", async () => {
    seed([{ id: "t1", agentSessionId: "from-hook" }, { id: "t2" }]);
    world.bindings = [
      { termKey: "t1", sessionId: "stale-or-other" },
      { termKey: "t2", sessionId: "sess-2" },
    ];
    expect(await adoptLiveResumeHandles()).toBe(1);
    expect(idOf("t1")).toBe("from-hook");
    expect(idOf("t2")).toBe("sess-2");
  });

  it("ignores Codex terminals: a claude id would break `codex resume`", async () => {
    seed([{ id: "t1", agent: "codex" }]);
    world.bindings = [{ termKey: "t1", sessionId: "claude-sess" }];
    expect(await adoptLiveResumeHandles()).toBe(0);
    expect(world.calls).toBe(0);
    expect(idOf("t1")).toBeUndefined();
  });

  it("ignores stopped terminals and terminals it does not know", async () => {
    seed([{ id: "t1", status: "stopped" }, { id: "t2" }]);
    world.bindings = [
      { termKey: "t1", sessionId: "sess-1" },
      { termKey: "ghost", sessionId: "sess-x" },
      { termKey: "t2", sessionId: "sess-2" },
    ];
    expect(await adoptLiveResumeHandles()).toBe(1);
    expect(idOf("t1")).toBeUndefined();
    expect(idOf("t2")).toBe("sess-2");
  });

  it("stays quiet when the backend call fails", async () => {
    seed([{ id: "t1" }]);
    world.fail = true;
    await expect(adoptLiveResumeHandles()).resolves.toBe(0);
    expect(idOf("t1")).toBeUndefined();
  });
});
