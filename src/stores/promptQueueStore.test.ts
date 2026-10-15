import { beforeEach, describe, expect, it } from "vitest";
import { queueFor, usePromptQueueStore } from "./promptQueueStore";

describe("promptQueueStore (Ctrl+Enter = queue for later)", () => {
  beforeEach(() => usePromptQueueStore.setState({ bySession: {} }));

  it("keeps order per session and hands prompts out one at a time", () => {
    const s = usePromptQueueStore.getState();
    s.enqueue("a", "first");
    s.enqueue("a", "second");
    s.enqueue("b", "other");
    expect(queueFor(usePromptQueueStore.getState().bySession, "a").map((q) => q.text)).toEqual([
      "first",
      "second",
    ]);
    expect(usePromptQueueStore.getState().shift("a")?.text).toBe("first");
    expect(queueFor(usePromptQueueStore.getState().bySession, "a").map((q) => q.text)).toEqual([
      "second",
    ]);
    expect(queueFor(usePromptQueueStore.getState().bySession, "b")).toHaveLength(1);
    expect(usePromptQueueStore.getState().shift("none")).toBeUndefined();
  });

  it("removes a single item and clears a session", () => {
    const s = usePromptQueueStore.getState();
    const x = s.enqueue("a", "x");
    s.enqueue("a", "y");
    usePromptQueueStore.getState().remove("a", x.id);
    expect(queueFor(usePromptQueueStore.getState().bySession, "a").map((q) => q.text)).toEqual([
      "y",
    ]);
    usePromptQueueStore.getState().clear("a");
    expect(queueFor(usePromptQueueStore.getState().bySession, "a")).toEqual([]);
    expect(queueFor({}, undefined)).toEqual([]);
  });
});
