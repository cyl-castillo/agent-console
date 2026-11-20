import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AssistantReply } from "../types/domain";

const world = vi.hoisted(() => ({
  calls: [] as {
    message: string;
    engine: string;
    resume: string | null;
    remember?: boolean;
    spoken?: boolean;
  }[],
  rated: [] as { id: string; rating: string | null }[],
  rateError: null as string | null,
  cleared: 0,
  proposeError: null as string | null,
  reply: null as AssistantReply | null,
  error: null as string | null,
  gate: null as Promise<void> | null,
  project: { root: "/repo" } as { root: string } | null,
  agent: undefined as string | undefined,
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("../ipc/tauri", () => ({
  ipc: {
    assistantAsk: async (
      message: string,
      engine: string,
      resume: string | null,
      remember?: boolean,
      spoken?: boolean,
    ) => {
      world.calls.push({ message, engine, resume, remember, spoken });
      if (world.gate) await world.gate;
      if (world.error) throw new Error(world.error);
      return world.reply;
    },
    assistantLogRate: async (id: string, rating: string | null) => {
      if (world.rateError) throw new Error(world.rateError);
      world.rated.push({ id, rating });
    },
    assistantLogClear: async () => {
      world.cleared += 1;
    },
    assistantLogSummary: async () => ({
      entries: world.cleared ? 0 : 3,
      failed: 0,
      ratedUp: 0,
      ratedDown: 0,
      path: "/data/log.jsonl",
      lastProposals: null,
    }),
    assistantPropose: async () => {
      if (world.proposeError) throw new Error(world.proposeError);
      return {
        at: 1,
        analyzed: 3,
        proposals: [{ kind: "guide", title: "T", rationale: "R", evidence: [] }],
      };
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
  world.rated = [];
  world.rateError = null;
  world.cleared = 0;
  world.proposeError = null;
  useAssistantStore.getState().reset();
  useAssistantStore.setState({
    remember: false,
    learning: null,
    proposing: false,
    learningError: null,
  });
  useAssistantStore.setState({ project: null });
});

describe("console assistant", () => {
  it("asks on the project's default engine and keeps the conversation by session id", async () => {
    world.agent = "codex";
    await useAssistantStore.getState().ask("  where is the ledger?  ");
    expect(world.calls).toMatchObject([
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
    expect(world.calls[1]).toMatchObject({ message: "open it", engine: "codex", resume: "s-1" });
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

  it("resolves to the reply's text, and to null when the turn fails or cannot run", async () => {
    expect(await useAssistantStore.getState().ask("where is the ledger?")).toBe(
      "It is under Proof.",
    );
    expect(await useAssistantStore.getState().ask("   ")).toBeNull();
    world.project = null;
    expect(await useAssistantStore.getState().ask("anyone?")).toBeNull();
  });

  describe("learning log", () => {
    const lastReply = () => {
      const all = useAssistantStore.getState().messages;
      return all[all.length - 1]!;
    };

    it("stores nothing until the human switches it on, and marks spoken questions", async () => {
      await useAssistantStore.getState().ask("one");
      expect(world.calls[0]).toMatchObject({ remember: false, spoken: false });
      expect(lastReply().logId).toBeUndefined();

      useAssistantStore.getState().setRemember(true);
      world.reply = reply({ logId: "L1" });
      await useAssistantStore.getState().ask("two", { spoken: true });
      expect(world.calls[1]).toMatchObject({ remember: true, spoken: true });
      expect(lastReply().logId).toBe("L1");
    });

    it("rates a logged reply, clears on the same rating, and rolls back when it cannot save", async () => {
      useAssistantStore.getState().setRemember(true);
      world.reply = reply({ logId: "L1" });
      await useAssistantStore.getState().ask("one");
      const id = lastReply().id;

      await useAssistantStore.getState().rate(id, "down");
      expect(lastReply().rating).toBe("down");
      await useAssistantStore.getState().rate(id, "down");
      expect(lastReply().rating).toBeUndefined();
      expect(world.rated).toEqual([
        { id: "L1", rating: "down" },
        { id: "L1", rating: null },
      ]);

      world.rateError = "disk full";
      await useAssistantStore.getState().rate(id, "up");
      expect(lastReply().rating).toBeUndefined();
      expect(useAssistantStore.getState().learningError).toContain("disk full");
    });

    it("does not rate a reply that was never logged", async () => {
      await useAssistantStore.getState().ask("one");
      await useAssistantStore.getState().rate(lastReply().id, "up");
      expect(world.rated).toEqual([]);
    });

    it("keeps the proposals it asked for and says why when the engine fails", async () => {
      await useAssistantStore.getState().refreshLearning();
      await useAssistantStore.getState().propose();
      // The summary is re-read after proposing; the mock's has none, which is
      // the on-disk truth the panel must show.
      expect(useAssistantStore.getState().proposing).toBe(false);
      expect(useAssistantStore.getState().learningError).toBeNull();

      world.proposeError = "engine timed out";
      await useAssistantStore.getState().propose();
      expect(useAssistantStore.getState().learningError).toContain("engine timed out");
      expect(useAssistantStore.getState().proposing).toBe(false);
    });

    it("clearing the log leaves nothing on screen pointing at a deleted turn", async () => {
      useAssistantStore.getState().setRemember(true);
      world.reply = reply({ logId: "L1" });
      await useAssistantStore.getState().ask("one");
      await useAssistantStore.getState().rate(lastReply().id, "up");

      await useAssistantStore.getState().clearLog();
      expect(world.cleared).toBe(1);
      expect(lastReply().logId).toBeUndefined();
      expect(lastReply().rating).toBeUndefined();
      expect(useAssistantStore.getState().learning?.entries).toBe(0);
    });
  });
});
