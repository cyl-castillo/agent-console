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

import { jqlForRole } from "../lib/jira";
import { useRoleStore } from "./roleStore";

beforeEach(() => {
  storage.map.clear();
  useRoleStore.setState({ roles: {}, jqls: {}, sprintScopes: {} });
});

describe("per-project role", () => {
  it("defaults to developer and survives a fresh store via localStorage", () => {
    expect(useRoleStore.getState().roleFor("/repo")).toBe("developer");
    useRoleStore.getState().setRoleFor("/repo", "qa");
    useRoleStore.setState({ roles: {}, jqls: {} });
    expect(useRoleStore.getState().roleFor("/repo")).toBe("qa");
    // Junk in storage degrades to developer.
    storage.map.set("agent-console:role:/tampered", "hacker");
    expect(useRoleStore.getState().roleFor("/tampered")).toBe("developer");
  });

  it("keeps roles per project", () => {
    useRoleStore.getState().setRoleFor("/a", "pm");
    expect(useRoleStore.getState().roleFor("/a")).toBe("pm");
    expect(useRoleStore.getState().roleFor("/b")).toBe("developer");
  });
});

describe("per-(project, role) JQL override", () => {
  it("falls back to the role preset and reports custom state honestly", () => {
    const s = useRoleStore.getState();
    expect(s.jqlFor("/repo", "qa")).toBe(jqlForRole("qa"));
    expect(s.hasCustomJql("/repo", "qa")).toBe(false);

    s.setJqlFor("/repo", "qa", 'status = "Verificación"');
    expect(useRoleStore.getState().jqlFor("/repo", "qa")).toBe('status = "Verificación"');
    expect(useRoleStore.getState().hasCustomJql("/repo", "qa")).toBe(true);
    // The override is scoped to that role.
    expect(useRoleStore.getState().jqlFor("/repo", "pm")).toBe(jqlForRole("pm"));
  });

  it("saving the preset itself (or clearing) removes the override", () => {
    const s = useRoleStore.getState();
    s.setJqlFor("/repo", "qa", "custom");
    s.setJqlFor("/repo", "qa", jqlForRole("qa"));
    expect(useRoleStore.getState().hasCustomJql("/repo", "qa")).toBe(false);
    s.setJqlFor("/repo", "qa", "custom2");
    s.setJqlFor("/repo", "qa", null);
    expect(useRoleStore.getState().jqlFor("/repo", "qa")).toBe(jqlForRole("qa"));
  });
});

describe("per-project sprint scope", () => {
  it("defaults to 'all', persists per project, and 'all' cleans storage up", () => {
    const s = useRoleStore.getState();
    expect(s.sprintScopeFor("/repo")).toBe("all");

    s.setSprintScopeFor("/repo", "active");
    expect(useRoleStore.getState().sprintScopeFor("/repo")).toBe("active");
    expect(useRoleStore.getState().sprintScopeFor("/other")).toBe("all");
    // Survives a fresh store via localStorage.
    useRoleStore.setState({ sprintScopes: {} });
    expect(useRoleStore.getState().sprintScopeFor("/repo")).toBe("active");

    useRoleStore.getState().setSprintScopeFor("/repo", "all");
    expect(storage.map.has("agent-console:jira-sprint:/repo")).toBe(false);
    // Junk in storage degrades to 'all'.
    storage.map.set("agent-console:jira-sprint:/tampered", "everything");
    expect(useRoleStore.getState().sprintScopeFor("/tampered")).toBe("all");
  });
});
