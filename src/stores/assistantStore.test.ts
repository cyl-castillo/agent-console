import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AssistantReply } from "../types/domain";

const world = vi.hoisted(() => ({
  calls: [] as { message: string; engine: string; resume: string | null }[],
  reply: null as AssistantReply | null,
  error: null as string | null,
  gate: null as Promise<void> | null,
  project: { root: "/repo" } as { root: string } | null,
  agent: undefined as string | undefined,
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("../ipc/tauri", () => ({
  ipc: {
    assistantAsk: async (message: string, engine: string, resume: string | null) => {
      world.calls.push({ message, engine, resume });
      if (world.gate) await world.gate;
      if (world.error) throw new Error(world.error);
      return world.reply;
    },
  },
}));
vi.mock("./sessionStore", () => ({
  useSessionStore: { getState: () => ({ project: world.project }) },
}));
vi.mock("./modelStore", () => ({
  useModelStore: { getState: () => ({ defaultAgentFor: () => world.agent }) },
}));

import { activityLine, useAssistantStore } from "./assistantStore";

const reply = (over: Partial<AssistantReply> = {}): AssistantReply => ({
  text: "It is under Proof.",
  sessionId: "s-1",
  tokens: 120,
  costUsd: 0,
  toolsAttached: true,
  ...over,
});

beforeEach(() => {
  world.calls = [];
  world.reply = reply();
  world.error = null;
  world.gate = null;
  world.project = { root: "/repo" };
  world.agent = undefined;
  useAssistantStore.getState().reset();
  useAssistantStore.setState({ project: null });
});

describe("console assistant", () => {
  it("asks on the project's default engine and keeps the conversation by session id", async () => {
    world.agent = "codex";
    await useAssistantStore.getState().ask("  where is the ledger?  ");
    expect(world.calls).toEqual([
      { message: "where is the ledger?", engine: "codex", resume: null },
    ]);
    let s = useAssistantStore.getState();
    expect(s.messages.map((m) => [m.role, m.text])).toEqual([
      ["user", "where is the ledger?"],
      ["assistant", "It is under Proof."],
    ]);
    expect(s.status).toBe("idle");
    expect(s.tokens).toBe(120);

    // The default changes mid-conversation: the conversation stays on its engine.
    world.agent = "claude";
    world.reply = reply({ sessionId: null, text: "Done." });
    await useAssistantStore.getState().ask("open it");
    expect(world.calls[1]).toEqual({ message: "open it", engine: "codex", resume: "s-1" });
    s = useAssistantStore.getState();
    expect(s.sessionId).toBe("s-1");
    expect(s.tokens).toBe(240);
  });

  it("falls back to Claude when the project has no default agent", async () => {
    await useAssistantStore.getState().ask("hola");
    expect(world.calls[0]!.engine).toBe("claude");
  });

  it("shows a failure as its own message and stays usable", async () => {
    world.error = "failed to spawn `claude`";
    await useAssistantStore.getState().ask("hola");
    const s = useAssistantStore.getState();
    expect(s.messages[s.messages.length - 1]).toMatchObject({
      role: "error",
      text: "failed to spawn `claude`",
    });
    expect(s.status).toBe("idle");
    expect(s.sessionId).toBeNull();
  });

  it("ignores empty input, a second ask while working, and a closed project", async () => {
    await useAssistantStore.getState().ask("   ");
    expect(world.calls).toHaveLength(0);

    let release = () => {};
    world.gate = new Promise<void>((r) => (release = r));
    const first = useAssistantStore.getState().ask("one");
    await useAssistantStore.getState().ask("two");
    expect(world.calls.map((c) => c.message)).toEqual(["one"]);
    release();
    await first;

    world.project = null;
    world.gate = null;
    await useAssistantStore.getState().ask("three");
    expect(world.calls).toHaveLength(1);
  });

  it("drops a reply that lands after the conversation was reset", async () => {
    let release = () => {};
    world.gate = new Promise<void>((r) => (release = r));
    const pending = useAssistantStore.getState().ask("one");
    useAssistantStore.getState().reset();
    release();
    await pending;
    const s = useAssistantStore.getState();
    expect(s.messages).toEqual([]);
    expect(s.sessionId).toBeNull();
    expect(s.status).toBe("idle");
  });

  it("starts over when the project changes", async () => {
    await useAssistantStore.getState().ask("one");
    world.project = { root: "/other" };
    await useAssistantStore.getState().ask("two");
    expect(world.calls[1]!.resume).toBeNull();
    expect(useAssistantStore.getState().messages.map((m) => m.text)).toEqual([
      "two",
      "It is under Proof.",
    ]);
  });

  it("reports the tool in flight only while a turn is working", () => {
    expect(activityLine("tool", "mcp__agent_console__invoke_action")).toBe("Using invoke action…");
    expect(activityLine("thinking", "")).toBe("Thinking…");
    expect(activityLine("other", "")).toBeNull();
    useAssistantStore.getState()._onActivity("thinking", "");
    expect(useAssistantStore.getState().activity).toBeNull();
    useAssistantStore.setState({ status: "working" });
    useAssistantStore.getState()._onActivity("tool", "console_state");
    expect(useAssistantStore.getState().activity).toBe("Using console state…");
  });
});
