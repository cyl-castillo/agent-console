import { beforeEach, describe, expect, it, vi } from "vitest";

const world = vi.hoisted(() => ({
  status: { configured: false, siteUrl: "", email: "" },
  statusError: null as string | null,
  connectError: null as string | null,
  disconnectError: null as string | null,
  issues: [] as { key: string }[],
  issuesError: null as string | null,
  logged: [] as string[],
  logError: null as string | null,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    jiraLogWork: async (issueKey: string, duration: string) => {
      if (world.logError) throw new Error(world.logError);
      world.logged.push(`${issueKey}:${duration}`);
      return "1h 30m";
    },
    jiraStatus: async () => {
      if (world.statusError) throw new Error(world.statusError);
      return world.status;
    },
    jiraConnect: async () => {
      if (world.connectError) throw new Error(world.connectError);
      world.status = { configured: true, siteUrl: "https://x.atlassian.net", email: "a@b.c" };
    },
    jiraDisconnect: async () => {
      if (world.disconnectError) throw new Error(world.disconnectError);
    },
    jiraListIssues: async () => {
      if (world.issuesError) throw new Error(world.issuesError);
      return world.issues;
    },
  },
}));

import { useJiraStore } from "./jiraStore";

beforeEach(() => {
  world.status = { configured: false, siteUrl: "", email: "" };
  world.statusError = null;
  world.connectError = null;
  world.disconnectError = null;
  world.issues = [];
  world.issuesError = null;
  world.logged = [];
  world.logError = null;
  useJiraStore.setState({
    status: null,
    issues: [],
    loadingStatus: false,
    loadingIssues: false,
    connecting: false,
    connectError: null,
    issuesError: null,
    logError: null,
  });
});

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("jira connection", () => {
  it("loadStatus on a configured site auto-fetches the issue queue", async () => {
    world.status = { configured: true, siteUrl: "https://x.atlassian.net", email: "a@b.c" };
    world.issues = [{ key: "FIX-1" }];
    await useJiraStore.getState().loadStatus();
    await settle();
    expect(useJiraStore.getState().issues).toEqual([{ key: "FIX-1" }]);
  });

  it("a broken status read degrades to 'not configured' instead of crashing the panel", async () => {
    world.statusError = "keychain locked";
    await useJiraStore.getState().loadStatus();
    const s = useJiraStore.getState();
    expect(s.loadingStatus).toBe(false);
    expect(s.status?.configured).toBe(false);
  });

  it("connect success reloads status and reports true", async () => {
    world.issues = [{ key: "FIX-2" }];
    const ok = await useJiraStore.getState().connect("https://x.atlassian.net", "a@b.c", "tok");
    await settle();
    expect(ok).toBe(true);
    const s = useJiraStore.getState();
    expect(s.status?.configured).toBe(true);
    expect(s.issues).toEqual([{ key: "FIX-2" }]);
  });

  it("connect failure surfaces in the form and reports false", async () => {
    world.connectError = "401 unauthorized";
    const ok = await useJiraStore.getState().connect("https://x.atlassian.net", "a@b.c", "bad");
    expect(ok).toBe(false);
    expect(useJiraStore.getState().connectError).toContain("401");
  });

  it("connect is re-entrancy guarded", async () => {
    useJiraStore.setState({ connecting: true });
    const ok = await useJiraStore.getState().connect("https://x", "a@b.c", "tok");
    expect(ok).toBe(false);
  });

  it("disconnect clears local state even when the backend call fails", async () => {
    useJiraStore.setState({
      status: { configured: true, siteUrl: "https://x", email: "a@b.c" },
      issues: [{ key: "FIX-1" }] as never,
    });
    world.disconnectError = "network";
    await useJiraStore.getState().disconnect();
    const s = useJiraStore.getState();
    expect(s.status?.configured).toBe(false);
    expect(s.issues).toEqual([]);
  });

  it("issue-fetch failures land in issuesError, separate from the connect form", async () => {
    world.issuesError = "jql rejected";
    await useJiraStore.getState().refreshIssues();
    const s = useJiraStore.getState();
    expect(s.issuesError).toContain("jql rejected");
    expect(s.connectError).toBeNull();
  });
});
describe("worklog", () => {
  it("logWork resolves to the normalized label on success", async () => {
    const label = await useJiraStore.getState().logWork("FIX-1", "90m", "2026-07-22");
    expect(label).toBe("1h 30m");
    expect(world.logged).toEqual(["FIX-1:90m"]);
    expect(useJiraStore.getState().logError).toBeNull();
  });

  it("logWork failure returns null and surfaces the error", async () => {
    world.logError = "401 unauthorized";
    const label = await useJiraStore.getState().logWork("FIX-1", "1h", "2026-07-22");
    expect(label).toBeNull();
    expect(useJiraStore.getState().logError).toContain("401");
  });
});
