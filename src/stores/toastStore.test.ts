import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The store schedules auto-dismiss via window.setTimeout; forward to the
// (possibly fake) global timer at call time.
vi.hoisted(() => {
  vi.stubGlobal("window", {
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
  });
});

import { useToastStore } from "./toastStore";

beforeEach(() => {
  vi.useFakeTimers();
  useToastStore.setState({ toasts: [] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("toasts", () => {
  it("info toasts auto-dismiss; error toasts stay until acted on", () => {
    useToastStore.getState().show("saved", "success");
    useToastStore.getState().show("boom", "error");
    expect(useToastStore.getState().toasts).toHaveLength(2);

    vi.advanceTimersByTime(3000);
    const left = useToastStore.getState().toasts;
    expect(left).toHaveLength(1);
    expect(left[0].message).toBe("boom");

    useToastStore.getState().dismiss(left[0].id);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("keeps only the 4 most recent toasts", () => {
    for (let i = 1; i <= 6; i++) useToastStore.getState().show(`t${i}`, "error");
    const msgs = useToastStore.getState().toasts.map((t) => t.message);
    expect(msgs).toEqual(["t3", "t4", "t5", "t6"]);
  });

  it("defaults to the transient info tone", () => {
    useToastStore.getState().show("hey");
    expect(useToastStore.getState().toasts[0].tone).toBe("info");
    vi.advanceTimersByTime(3000);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });
});
