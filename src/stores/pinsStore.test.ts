import { describe, expect, it, vi } from "vitest";

// The store reads localStorage at module-init — stub before import.
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

const KEY = "agent-console.pinned-skills.v1";

import { pinKey, usePinsStore } from "./pinsStore";

describe("pinned skills", () => {
  it("pin keys disambiguate kind and source, not just name", () => {
    expect(pinKey("skill", "project", "deploy")).not.toBe(pinKey("skill", "user", "deploy"));
  });

  it("toggle pins, persists, and unpins", () => {
    const k = pinKey("skill", "project", "deploy");
    usePinsStore.getState().toggle(k);
    expect(usePinsStore.getState().isPinned(k)).toBe(true);
    expect(JSON.parse(storage.map.get(KEY)!)).toContain(k);

    usePinsStore.getState().toggle(k);
    expect(usePinsStore.getState().isPinned(k)).toBe(false);
    expect(JSON.parse(storage.map.get(KEY)!)).toEqual([]);
  });

  it("corrupt or non-array storage falls back to no pins", async () => {
    storage.map.set(KEY, "{not json");
    vi.resetModules();
    let fresh = await import("./pinsStore");
    expect(fresh.usePinsStore.getState().pinned.size).toBe(0);

    storage.map.set(KEY, JSON.stringify({ evil: true }));
    vi.resetModules();
    fresh = await import("./pinsStore");
    expect(fresh.usePinsStore.getState().pinned.size).toBe(0);
  });
});
