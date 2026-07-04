import { describe, expect, it, vi } from "vitest";

import type { ApprovalRequest } from "../types/domain";

// blockedSessionIds is pure, but the module it lives in touches the backend
// and sibling stores at import time — isolate those.
vi.mock("../ipc/tauri", () => ({
  ipc: { approvalRespond: vi.fn() },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("./agentStatusStore", () => ({
  useAgentStatusStore: { getState: () => ({ markActive: vi.fn() }) },
}));

import { blockedSessionIds, type SessionRef } from "./approvalStore";

function req(over: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "r1",
    ts: 1,
    sessionDir: "/home/u/.claude/projects/x",
    cwd: "/repo",
    tool: "Bash",
    input: {},
    ...over,
  };
}

function ses(id: string, cwd: string, status = "live"): SessionRef {
  return { id, cwd, status };
}

describe("blockedSessionIds (which session a pending approval belongs to)", () => {
  it("attributes by termId when the hook tagged the request", () => {
    const sessions = [ses("t1", "/repo"), ses("t2", "/repo")];
    const out = blockedSessionIds([req({ termId: "t2" })], sessions);
    expect(out).toEqual(new Set(["t2"]));
  });

  it("termId wins over cwd (two sessions share the cwd, only the tagged one is blocked)", () => {
    const sessions = [ses("t1", "/repo"), ses("t2", "/repo")];
    const out = blockedSessionIds([req({ termId: "t1", cwd: "/repo" })], sessions);
    expect(out).toEqual(new Set(["t1"]));
  });

  it("falls back to cwd when termId is missing and exactly one live session matches", () => {
    const sessions = [ses("t1", "/repo-a"), ses("t2", "/repo-b")];
    const out = blockedSessionIds([req({ cwd: "/repo-b" })], sessions);
    expect(out).toEqual(new Set(["t2"]));
  });

  it("attributes to nobody when the cwd is ambiguous (two live sessions, same checkout)", () => {
    const sessions = [ses("t1", "/repo"), ses("t2", "/repo")];
    const out = blockedSessionIds([req({ cwd: "/repo" })], sessions);
    expect(out.size).toBe(0);
  });

  it("ignores stopped sessions in the cwd fallback", () => {
    const sessions = [ses("t1", "/repo", "stopped"), ses("t2", "/repo")];
    const out = blockedSessionIds([req({ cwd: "/repo" })], sessions);
    expect(out).toEqual(new Set(["t2"]));
  });

  it("a stale termId (session already closed) falls back to cwd", () => {
    const sessions = [ses("t2", "/repo")];
    const out = blockedSessionIds([req({ termId: "gone", cwd: "/repo" })], sessions);
    expect(out).toEqual(new Set(["t2"]));
  });

  it("aggregates multiple requests across sessions", () => {
    const sessions = [ses("t1", "/a"), ses("t2", "/b")];
    const out = blockedSessionIds(
      [req({ id: "r1", termId: "t1" }), req({ id: "r2", cwd: "/b" })],
      sessions,
    );
    expect(out).toEqual(new Set(["t1", "t2"]));
  });

  it("empty queue blocks nobody", () => {
    expect(blockedSessionIds([], [ses("t1", "/repo")]).size).toBe(0);
  });
});
