import { beforeEach, describe, expect, it, vi } from "vitest";

const storage = vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      map.set(k, v);
    },
    removeItem: (k: string) => {
      map.delete(k);
    },
  });
  return { map };
});

import { useModelStore } from "./modelStore";

beforeEach(() => {
  storage.map.clear();
  useModelStore.setState({ modelDefaults: {}, agentDefaults: {} });
});

describe("per-project model/agent defaults", () => {
  it("remembers the model per (project, agent) — Claude's choice can't leak into Codex", () => {
    useModelStore.getState().setDefaultFor("/repo", "claude", "opus");
    expect(useModelStore.getState().defaultFor("/repo", "claude")).toBe("opus");
    expect(useModelStore.getState().defaultFor("/repo", "codex")).toBeUndefined();
    expect(useModelStore.getState().defaultFor("/other", "claude")).toBeUndefined();
  });

  it("survives a fresh store via localStorage", () => {
    useModelStore.getState().setDefaultFor("/repo", "claude", "sonnet");
    // Simulate a new window: caches empty, storage intact.
    useModelStore.setState({ modelDefaults: {}, agentDefaults: {} });
    expect(useModelStore.getState().defaultFor("/repo", "claude")).toBe("sonnet");
  });

  it("rejects models that could break the launch command — stored value is removed", () => {
    useModelStore.getState().setDefaultFor("/repo", "claude", "opus");
    useModelStore.getState().setDefaultFor("/repo", "claude", "opus; rm -rf /");
    expect(useModelStore.getState().defaultFor("/repo", "claude")).toBeUndefined();
    expect([...storage.map.keys()].some((k) => k.includes("model"))).toBe(false);
  });

  it("clearing with undefined removes the stored default", () => {
    useModelStore.getState().setDefaultFor("/repo", "claude", "opus");
    useModelStore.getState().setDefaultFor("/repo", "claude", undefined);
    expect(useModelStore.getState().defaultFor("/repo", "claude")).toBeUndefined();
  });

  it("remembers the agent per project and ignores junk in storage", () => {
    useModelStore.getState().setDefaultAgentFor("/repo", "codex");
    useModelStore.setState({ modelDefaults: {}, agentDefaults: {} });
    expect(useModelStore.getState().defaultAgentFor("/repo")).toBe("codex");

    storage.map.set("agent-console:agent:/tampered", "evil-agent");
    expect(useModelStore.getState().defaultAgentFor("/tampered")).toBeUndefined();
  });
});
