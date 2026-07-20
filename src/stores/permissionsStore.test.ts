import { beforeEach, describe, expect, it, vi } from "vitest";

import type { StoredRule } from "../types/domain";

const world = vi.hoisted(() => ({
  rules: [] as { scope: string; effect: string; raw: string }[],
  addError: null as string | null,
  removeError: null as string | null,
  snapshotCalls: 0,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    permissionsSnapshot: async () => {
      world.snapshotCalls++;
      return { rules: world.rules };
    },
    permissionsAdd: async (scope: string, effect: string, raw: string) => {
      if (world.addError) throw new Error(world.addError);
      const rule = { scope, effect, raw };
      world.rules.push(rule);
      return rule;
    },
    permissionsRemove: async (scope: string, effect: string, raw: string) => {
      if (world.removeError) throw new Error(world.removeError);
      world.rules = world.rules.filter(
        (r) => !(r.scope === scope && r.effect === effect && r.raw === raw),
      );
    },
  },
}));

import { usePermissionsStore } from "./permissionsStore";

beforeEach(() => {
  world.rules = [];
  world.addError = null;
  world.removeError = null;
  world.snapshotCalls = 0;
  usePermissionsStore.setState({ snapshot: null, loading: false, lastOp: null, error: null });
});

describe("permissions rules", () => {
  it("add records the op for undo and refreshes the snapshot", async () => {
    await usePermissionsStore.getState().add("project", "allow", "Bash(npm test)");
    const s = usePermissionsStore.getState();
    expect(s.lastOp).toEqual({
      kind: "add",
      scope: "project",
      effect: "allow",
      raw: "Bash(npm test)",
    });
    expect(s.snapshot?.rules).toHaveLength(1);
  });

  it("undo of an add removes the rule and cannot be undone twice", async () => {
    await usePermissionsStore.getState().add("project", "allow", "Bash(npm test)");
    await usePermissionsStore.getState().undo();
    expect(world.rules).toHaveLength(0);
    expect(usePermissionsStore.getState().lastOp).toBeNull();

    // Second undo: nothing to do, nothing breaks.
    await usePermissionsStore.getState().undo();
    expect(world.rules).toHaveLength(0);
  });

  it("undo of a remove restores the rule", async () => {
    await usePermissionsStore.getState().add("project", "deny", "WebFetch");
    await usePermissionsStore.getState().remove("project", "deny", "WebFetch");
    expect(world.rules).toHaveLength(0);
    await usePermissionsStore.getState().undo();
    expect(world.rules).toEqual([{ scope: "project", effect: "deny", raw: "WebFetch" }]);
  });

  it("move adds to the new scope before removing from the old (no window with zero copies)", async () => {
    await usePermissionsStore.getState().add("project", "allow", "Read");
    const from: StoredRule = { scope: "project", effect: "allow", raw: "Read" } as StoredRule;
    await usePermissionsStore.getState().move(from, "global");
    expect(world.rules).toEqual([{ scope: "global", effect: "allow", raw: "Read" }]);
    // Undo removes the NEW placement.
    expect(usePermissionsStore.getState().lastOp?.scope).toBe("global");

    // Same-scope move is a no-op.
    world.snapshotCalls = 0;
    await usePermissionsStore.getState().move({ ...from, scope: "global" } as StoredRule, "global");
    expect(world.snapshotCalls).toBe(0);
  });

  it("backend failures surface as error without corrupting lastOp", async () => {
    world.addError = "settings.json locked";
    const r = await usePermissionsStore.getState().add("project", "allow", "X");
    expect(r).toBeNull();
    const s = usePermissionsStore.getState();
    expect(s.error).toContain("settings.json locked");
    expect(s.lastOp).toBeNull();
  });
});
