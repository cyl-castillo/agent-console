import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/tauri", () => ({
  ipc: { sessionsSave: vi.fn(async () => {}), sessionsList: vi.fn(async () => []) },
}));

import { newSession } from "../app/actions";
import { useModelStore } from "../stores/modelStore";
import { preflightSettled, usePreflightStore } from "../stores/preflightStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import type { Preflight } from "../types/domain";
import { initialAgentArg, initialAgentFor, pickInitialAgent } from "./initialAgent";

const preflight = (found: string[]): Preflight => ({
  tools: ["claude", "node", "git", "codex", "opencode"].map((name) => ({
    name,
    found: found.includes(name),
    version: found.includes(name) ? "1.0.0" : null,
    fixCommand: null,
    fixNote: null,
  })),
  auth: [],
});

const ROOT = "/p";

beforeEach(() => {
  usePreflightStore.setState({ result: null, checking: false });
  useModelStore.setState({ agentDefaults: {} });
  useTerminalsStore.setState({ projectRoot: ROOT, sessions: [], activeId: null, ready: true });
  useSessionStore.setState({ project: { root: ROOT, name: "p" } as never });
});

describe("the engine of a session nobody picked one for", () => {
  it("is an engine that is installed when Claude is not", () => {
    expect(pickInitialAgent(undefined, preflight(["codex", "git"]))).toBe("codex");
    expect(pickInitialAgent(undefined, preflight(["opencode"]))).toBe("opencode");
    expect(pickInitialAgent(undefined, preflight(["codex", "opencode"]))).toBe("codex");
  });

  it("is Claude when Claude is installed, when nothing is, and before the check answers", () => {
    expect(pickInitialAgent(undefined, preflight(["claude", "codex", "opencode"]))).toBe("claude");
    expect(pickInitialAgent(undefined, preflight(["git"]))).toBe("claude");
    expect(pickInitialAgent(undefined, null)).toBe("claude");
  });

  it("is the one last used in the folder, whatever is installed", () => {
    expect(pickInitialAgent("opencode", preflight(["claude"]))).toBe("opencode");
    useModelStore.setState({ agentDefaults: { [ROOT]: "codex" } });
    usePreflightStore.setState({ result: preflight(["claude"]) });
    expect(initialAgentFor(ROOT)).toBe("codex");
  });

  it("leaves Claude unset on the session, as a session opened without a choice always was", () => {
    usePreflightStore.setState({ result: preflight(["claude", "codex"]) });
    expect(initialAgentArg(ROOT)).toBeUndefined();
    usePreflightStore.setState({ result: preflight(["opencode"]) });
    expect(initialAgentArg(ROOT)).toBe("opencode");
  });
});

describe("a new session from the palette or an empty state", () => {
  it("starts the installed engine on a machine without Claude", () => {
    usePreflightStore.setState({ result: preflight(["codex", "git"]) });
    newSession();
    expect(useTerminalsStore.getState().sessions.map((s) => s.agent)).toEqual(["codex"]);
  });

  it("starts the engine last used in the folder", () => {
    usePreflightStore.setState({ result: preflight(["claude", "codex", "opencode"]) });
    useModelStore.setState({ agentDefaults: { [ROOT]: "opencode" } });
    newSession();
    expect(useTerminalsStore.getState().sessions.map((s) => s.agent)).toEqual(["opencode"]);
  });

  it("still starts Claude, unset, where Claude is installed", () => {
    usePreflightStore.setState({ result: preflight(["claude", "codex"]) });
    newSession();
    expect(useTerminalsStore.getState().sessions.map((s) => s.agent)).toEqual([undefined]);
  });
});

describe("waiting for the setup check", () => {
  it("answers at once when the result is there or no check is running", async () => {
    expect(await preflightSettled()).toBeNull();
    const result = preflight(["codex"]);
    usePreflightStore.setState({ result, checking: true });
    expect(await preflightSettled()).toBe(result);
  });

  it("answers when the running check lands", async () => {
    usePreflightStore.setState({ result: null, checking: true });
    const settled = preflightSettled(5000);
    const result = preflight(["opencode"]);
    usePreflightStore.setState({ result, checking: false });
    expect(await settled).toBe(result);
  });

  it("gives up after the wait and answers with what there is", async () => {
    vi.useFakeTimers();
    usePreflightStore.setState({ result: null, checking: true });
    const settled = preflightSettled(3000);
    vi.advanceTimersByTime(3000);
    expect(await settled).toBeNull();
    vi.useRealTimers();
  });
});
