import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/// The store reads localStorage at import time; give the module a fresh stub
/// and a fresh import per test so each case starts from its own stored state.
function stubStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
  });
  return data;
}

async function freshStore() {
  vi.resetModules();
  return await import("./modulesStore");
}

beforeEach(() => {
  stubStorage();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("modules store", () => {
  it("defaults to everything enabled except experimental modules", async () => {
    const { useModulesStore, isTabEnabled } = await freshStore();
    expect(useModulesStore.getState().disabled).toEqual(["watch"]);
    expect(isTabEnabled("watch")).toBe(false);
    expect(useModulesStore.getState().isEnabled("notes")).toBe(true);
    expect(isTabEnabled("jira")).toBe(true);
    // Groupless tabs belong to no module and are always on.
    expect(isTabEnabled("transfer")).toBe(true);
  });

  it("toggling a module persists the disabled set and gates its tabs", async () => {
    const data = stubStorage();
    const { useModulesStore, isTabEnabled } = await freshStore();
    useModulesStore.getState().setEnabled("tasks", false);
    expect(isTabEnabled("jira")).toBe(false);
    expect(isTabEnabled("agenda")).toBe(false);
    expect(isTabEnabled("notes")).toBe(true);
    expect(JSON.parse(data.get("agent-console.modules.v1")!)).toEqual(["tasks"]);
    expect(useModulesStore.getState().disabled).toEqual(["watch", "tasks"]);
    useModulesStore.getState().setEnabled("tasks", true);
    expect(isTabEnabled("jira")).toBe(true);
    expect(JSON.parse(data.get("agent-console.modules.v1")!)).toEqual([]);
  });

  it("trust is locked on — setEnabled(false) is a no-op", async () => {
    const { useModulesStore, isTabEnabled } = await freshStore();
    useModulesStore.getState().setEnabled("trust", false);
    expect(useModulesStore.getState().isEnabled("trust")).toBe(true);
    expect(isTabEnabled("permissions")).toBe(true);
    expect(isTabEnabled("vault")).toBe(true);
  });

  it("ignores unknown and locked keys found in storage", async () => {
    stubStorage({
      "agent-console.modules.v1": JSON.stringify(["trust", "gone-module", "room"]),
    });
    const { useModulesStore } = await freshStore();
    expect(useModulesStore.getState().disabled).toEqual(["room", "watch"]);
  });

  it("an experimental module is born off, opts in, and stays in after a reload", async () => {
    const data = stubStorage();
    let store = await freshStore();
    expect(store.isTabEnabled("watch")).toBe(false);
    store.useModulesStore.getState().setEnabled("watch", true);
    expect(store.isTabEnabled("watch")).toBe(true);
    expect(JSON.parse(data.get("agent-console.modules.optin.v1")!)).toEqual(["watch"]);
    // The off-list never learns about experimental modules.
    expect(data.get("agent-console.modules.v1")).toBeUndefined();
    store = await freshStore();
    expect(store.isTabEnabled("watch")).toBe(true);
    store.useModulesStore.getState().setEnabled("watch", false);
    expect(store.isTabEnabled("watch")).toBe(false);
    expect(JSON.parse(data.get("agent-console.modules.optin.v1")!)).toEqual([]);
    // A non-experimental key in the opt-in list is ignored.
    data.set("agent-console.modules.optin.v1", JSON.stringify(["room", "nope"]));
    store = await freshStore();
    expect(store.useModulesStore.getState().disabled).toEqual(["watch"]);
  });

  it("survives corrupt storage and a missing localStorage", async () => {
    stubStorage({ "agent-console.modules.v1": "not json{" });
    let store = await freshStore();
    expect(store.useModulesStore.getState().disabled).toEqual(["watch"]);
    vi.unstubAllGlobals(); // node env: bare localStorage access now throws
    store = await freshStore();
    expect(store.useModulesStore.getState().disabled).toEqual(["watch"]);
    expect(() => store.useModulesStore.getState().setEnabled("room", false)).not.toThrow();
  });

  it("firstEnabledTab walks the strip order and skips disabled groups", async () => {
    const { useModulesStore, firstEnabledTab } = await freshStore();
    expect(firstEnabledTab()).toBe("jira"); // tasks is the first group
    useModulesStore.getState().setEnabled("tasks", false);
    expect(firstEnabledTab()).toBe("teams");
    useModulesStore.getState().setEnabled("teams", false);
    expect(firstEnabledTab()).toBe("notes");
  });
});
