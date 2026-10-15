import { beforeEach, describe, expect, it } from "vitest";

// localStorage stand-in: the store persists on every change.
const mem = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => mem.clear(),
  },
});

import { COLUMN_DEFAULTS, COLUMN_LIMITS, useShellStore } from "./shellStore";

describe("shellStore column widths", () => {
  beforeEach(() => {
    mem.clear();
    useShellStore.getState().setWidth("list", null);
    useShellStore.getState().setWidth("inspector", null);
  });

  it("defaults, clamps to the column's limits and persists", () => {
    const s = useShellStore.getState();
    expect(s.listWidth).toBe(COLUMN_DEFAULTS.list);
    expect(s.inspectorWidth).toBe(COLUMN_DEFAULTS.inspector);
    s.setWidth("list", 10_000);
    expect(useShellStore.getState().listWidth).toBe(COLUMN_LIMITS.list[1]);
    s.setWidth("inspector", 1);
    expect(useShellStore.getState().inspectorWidth).toBe(COLUMN_LIMITS.inspector[0]);
    s.setWidth("list", 301.4);
    expect(useShellStore.getState().listWidth).toBe(301);
    const persisted = JSON.parse(mem.get("agent-console:shell-v2") ?? "{}");
    expect(persisted.listWidth).toBe(301);
    expect(persisted.inspectorWidth).toBe(COLUMN_LIMITS.inspector[0]);
  });

  it("null resets to the default (double-click on the handle)", () => {
    useShellStore.getState().setWidth("inspector", 500);
    useShellStore.getState().setWidth("inspector", null);
    expect(useShellStore.getState().inspectorWidth).toBe(COLUMN_DEFAULTS.inspector);
  });
});
