import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/tauri", () => ({ ipc: { consoleReply: vi.fn() } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("./createSession", () => ({
  createSession: vi.fn(async () => {
    const { useTerminalsStore } = await import("../stores/terminalsStore");
    useTerminalsStore.setState((s) => ({
      sessions: [
        ...s.sessions,
        {
          id: "sess-new",
          name: "shell 2",
          cwd: "/p",
          createdAtMs: 1,
          initialScrollback: "",
          liveScrollback: "",
          status: "live",
          archived: false,
        } as never,
      ],
    }));
    return true;
  }),
}));

import { actionAllowed, handleConsoleCall } from "./consoleMcpHost";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useApprovalStore } from "../stores/approvalStore";

const call = (tool: string, args: Record<string, unknown> = {}) => ({
  id: 1,
  caller: "test-agent",
  tool,
  arguments: args,
});

describe("the console answers an agent's MCP calls from its stores", () => {
  beforeEach(() => {
    useSessionStore.setState({
      project: { root: "/p", name: "p", language: null, framework: null },
    });
    useTerminalsStore.setState({
      sessions: [
        {
          id: "sess-a",
          name: "fix auth",
          cwd: "/p",
          createdAtMs: 1,
          initialScrollback: "",
          liveScrollback: "",
          status: "live",
          archived: false,
        } as never,
      ],
      activeId: "sess-a",
    });
    useApprovalStore.setState({ queue: [] });
  });

  it("only harmless actions are open to agents, on both sides", () => {
    expect(actionAllowed("nav.inbox")).toBe(true);
    expect(actionAllowed("session.close")).toBe(false);
    expect(actionAllowed("git.push")).toBe(false);
  });

  it("state and actions come from the same builders the window bridge uses", async () => {
    const st = await handleConsoleCall(call("console_state"));
    expect(st.ok && (st.result as { project: string }).project).toBe("/p");
    const acts = await handleConsoleCall(call("list_actions"));
    expect(acts.ok && (acts.result as { id: string }[]).some((a) => a.id === "nav.proof")).toBe(
      true,
    );
  });

  it("refuses unknown and human-only actions", async () => {
    expect(await handleConsoleCall(call("invoke_action", { id: "git.push" }))).toEqual({
      ok: false,
      error: "Action git.push stays a human gesture",
    });
    expect((await handleConsoleCall(call("invoke_action", { id: "nope" }))).ok).toBe(false);
  });

  it("lists sessions with their attention state", async () => {
    useApprovalStore.setState({
      queue: [
        { id: "r", ts: 0, sessionDir: "", cwd: "/p", tool: "Bash", input: {}, termId: "sess-a" },
      ],
    });
    const res = await handleConsoleCall(call("list_sessions"));
    const sessions = res.ok
      ? (res.result as { sessions: { id: string; attention: string }[] }).sessions
      : [];
    expect(sessions[0]).toMatchObject({ id: "sess-a", attention: "needs-you" });
  });

  it("opens a session and names it", async () => {
    const res = await handleConsoleCall(call("new_session", { name: "docs", agent: "claude" }));
    expect(res).toEqual({ ok: true, result: { id: "sess-new", name: "docs", agent: "claude" } });
    expect(useTerminalsStore.getState().sessions.find((s) => s.id === "sess-new")?.name).toBe(
      "docs",
    );
  });

  it("send_prompt needs a live session", async () => {
    useTerminalsStore.setState((s) => ({
      sessions: s.sessions.map((x) => ({ ...x, status: "stopped" as const })),
    }));
    expect(
      (await handleConsoleCall(call("send_prompt", { session: "sess-a", text: "hi" }))).ok,
    ).toBe(false);
  });
});
