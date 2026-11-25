import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the Tauri IPC layer at load time; nothing here calls it.
vi.mock("../ipc/tauri", () => ({ ipc: {} }));

const show = vi.fn();
vi.mock("../stores/toastStore", () => ({
  useToastStore: { getState: () => ({ show }) },
}));

const dispatchEvent = vi.hoisted(() => {
  const dispatchEvent = vi.fn();
  vi.stubGlobal("window", { dispatchEvent });
  return dispatchEvent;
});

import { startInstallSession } from "./installSession";
import { startLoginSession } from "./loginSession";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import type { Project } from "../types/domain";

const sessions = () => useTerminalsStore.getState().sessions;
const openProject = () =>
  useSessionStore.setState({ project: { root: "/repo", name: "repo" } as Project });

describe("a terminal the doctor opens is one the person can reach", () => {
  beforeEach(() => {
    show.mockReset();
    dispatchEvent.mockReset();
    useSessionStore.setState({ project: null });
    useTerminalsStore.setState({ sessions: [], activeId: null });
  });

  it("opens no install terminal without a project: nothing would render it", async () => {
    const opened = await startInstallSession("codex", "npm install -g @openai/codex");

    expect(opened).toBe(false);
    expect(sessions()).toHaveLength(0);
    expect(dispatchEvent).not.toHaveBeenCalled();
    expect(show).toHaveBeenCalledTimes(1);
    expect(show.mock.calls[0][0]).toMatch(/open a project folder first/i);
  });

  it("opens no login terminal without a project either", async () => {
    const opened = await startLoginSession("codex");

    expect(opened).toBe(false);
    expect(sessions()).toHaveLength(0);
    expect(show.mock.calls[0][0]).toMatch(/open a project folder first/i);
  });

  it("types the installer into a session of the open project, unsent", async () => {
    openProject();
    const opened = await startInstallSession("codex", "npm install -g @openai/codex");

    expect(opened).toBe(true);
    expect(sessions()).toHaveLength(1);
    const [session] = sessions();
    expect(session.cwd).toBe("/repo");
    expect(session.name).toBe("install codex");
    // A plain shell with the command typed: no agent is launched in it.
    expect(session.shellCmd).toBe("npm install -g @openai/codex");
    expect(session.agent).toBeUndefined();
    expect(useTerminalsStore.getState().activeId).toBe(session.id);
    expect(dispatchEvent).toHaveBeenCalledTimes(1);
  });

  it("opens the login flow in a session of the open project", async () => {
    openProject();
    const opened = await startLoginSession("codex");

    expect(opened).toBe(true);
    const [session] = sessions();
    expect(session.cwd).toBe("/repo");
    expect(session.agent).toBe("codex");
    expect(session.loginOnly).toBe(true);
  });
});
