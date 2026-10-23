import { beforeEach, describe, expect, it, vi } from "vitest";

// rewindToTurn rewrites a working tree and spawns a session resuming a forked
// conversation — isolate the backend and the stores it touches.
vi.mock("../ipc/tauri", () => ({
  ipc: {
    turnRewind: vi.fn(),
    testigoList: vi.fn().mockResolvedValue([]),
    testigoVerify: vi.fn().mockResolvedValue(null),
    testigoGetSettings: vi.fn().mockResolvedValue(null),
    testigoExportAat: vi.fn(),
    testigoVerifyAat: vi.fn(),
    ledgerRootFor: vi.fn(),
  },
}));
const refresh = vi.fn().mockResolvedValue(undefined);
vi.mock("./changesStore", () => ({
  useChangesStore: { getState: () => ({ refresh }) },
}));
const showToast = vi.fn();
vi.mock("./toastStore", () => ({
  useToastStore: { getState: () => ({ show: showToast }) },
}));

import { ipc } from "../ipc/tauri";
import { summarizeCases, buildTimeline, useProofStore, type TimelineTurn } from "./proofStore";
import { useTerminalsStore, type TerminalSession } from "./terminalsStore";
import type { ProofEvent } from "../types/domain";

const mockRewind = vi.mocked(ipc.turnRewind);

function ev(partial: Partial<ProofEvent>): ProofEvent {
  return {
    seq: 0,
    ts: 0,
    caseId: "term:t1",
    kind: "prompt",
    actor: "human",
    payload: {},
    prevHash: "",
    hash: "",
    ...partial,
  };
}

describe("summarizeCases", () => {
  it("groups by case, counts turns/approvals, sorts by recency", () => {
    const cases = summarizeCases([
      ev({ seq: 0, ts: 1, caseId: "jira:FIXY-1", kind: "case_link", actor: "system" }),
      ev({ seq: 1, ts: 2, caseId: "jira:FIXY-1", kind: "prompt" }),
      ev({ seq: 2, ts: 3, caseId: "jira:FIXY-1", kind: "approval_decision" }),
      ev({ seq: 3, ts: 4, caseId: "jira:FIXY-1", kind: "turn_end", actor: "agent" }),
      ev({ seq: 4, ts: 9, caseId: "term:t2", kind: "prompt" }),
    ]);
    expect(cases.map((c) => c.caseId)).toEqual(["term:t2", "jira:FIXY-1"]);
    const fixy = cases[1];
    expect(fixy.events).toBe(4);
    expect(fixy.turns).toBe(1);
    expect(fixy.approvals).toBe(1);
    expect(fixy.lastTs).toBe(4);
  });

  it("empty ledger produces no cases", () => {
    expect(summarizeCases([])).toEqual([]);
  });
});

describe("buildTimeline", () => {
  it("D10: a reviewer verdict and the human's answer attach to the reviewed turn", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 1, kind: "prompt", turnId: "t1", payload: { prompt: "go" } }),
      ev({ seq: 2, ts: 2, kind: "turn_end", turnId: "t1", payload: {} }),
      ev({
        seq: 3,
        ts: 3,
        kind: "review_verdict",
        turnId: "t1",
        actor: "agent:reviewer:codex",
        payload: { verdict: "block", findings: [{ title: "x" }, { title: "y" }] },
      }),
      ev({
        seq: 4,
        ts: 4,
        kind: "review_decision",
        turnId: "t1",
        actor: "human",
        payload: { decision: "override", reason: "known flake" },
      }),
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0].review).toEqual({
      verdict: "block",
      findings: 2,
      reviewer: "agent:reviewer:codex",
      decision: "override",
    });
  });

  it("D5: a budget pause and its answer attach to the turn they cut", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 1, kind: "prompt", turnId: "t1", payload: { prompt: "go" } }),
      ev({
        seq: 2,
        ts: 2,
        kind: "budget_reached",
        turnId: "t1",
        actor: "console",
        payload: { spentUsd: 12.3, capUsd: 10, action: "interrupted" },
      }),
    ]);
    expect(turns[0].budget).toEqual({ spentUsd: 12.3, capUsd: 10 });
    const decided = buildTimeline([
      ev({ seq: 1, ts: 1, kind: "prompt", turnId: "t1", payload: { prompt: "go" } }),
      ev({
        seq: 2,
        ts: 2,
        kind: "budget_reached",
        turnId: "t1",
        payload: { spentUsd: 12.3, capUsd: 10 },
      }),
      ev({
        seq: 3,
        ts: 3,
        kind: "budget_decision",
        turnId: "t1",
        actor: "human",
        payload: { decision: "raise", spentUsd: 12.3, capUsd: 10, newCapUsd: 15 },
      }),
    ]);
    expect(decided[0].budget).toEqual({
      spentUsd: 12.3,
      capUsd: 10,
      decision: "raise",
      newCapUsd: 15,
    });
  });

  it("folds a case's events into turns with approvals, results and diff", () => {
    const turns = buildTimeline([
      ev({
        seq: 1,
        ts: 10,
        kind: "prompt",
        turnId: "T1",
        payload: { prompt: "do it", skill: "deploy" },
      }),
      ev({
        seq: 2,
        ts: 11,
        kind: "approval_decision",
        turnId: "T1",
        payload: { tool: "Bash", decision: "allow", reason: "ok" },
      }),
      ev({ seq: 3, ts: 12, kind: "tool_result", turnId: "T1", payload: { tool: "Bash" } }),
      ev({
        seq: 4,
        ts: 13,
        kind: "turn_end",
        turnId: "T1",
        payload: { filesChanged: [{ status: "M", path: "a.ts" }], filesTruncated: false },
      }),
      ev({ seq: 5, ts: 20, kind: "prompt", turnId: "T2", payload: { prompt: "next" } }),
    ]);
    expect(turns).toHaveLength(2);
    const t1 = turns[0];
    expect(t1.prompt).toBe("do it");
    expect(t1.skill).toBe("deploy");
    expect(t1.approvals).toEqual([{ tool: "Bash", decision: "allow", reason: "ok" }]);
    expect(t1.toolResults).toBe(1);
    expect(t1.files).toEqual([{ status: "M", path: "a.ts" }]);
    expect(t1.endTs).toBe(13);
    expect(turns[1].endTs).toBeNull();
  });

  it("carries the agent's closing words, and stays empty when the CLI sends none", () => {
    const withSummary = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "do it" } }),
      ev({
        seq: 2,
        ts: 13,
        kind: "turn_end",
        turnId: "T1",
        payload: { summary: "Fixed the retry loop", summaryTruncated: true },
      }),
    ]);
    expect(withSummary[0].summary).toBe("Fixed the retry loop");
    expect(withSummary[0].summaryTruncated).toBe(true);

    // Codex (and Claude before 2.1.47) close the turn without it — the turn
    // renders exactly as it did before, no empty "↳" line.
    const without = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T2", payload: { prompt: "do it" } }),
      ev({ seq: 2, ts: 13, kind: "turn_end", turnId: "T2", payload: { postSha: "abc" } }),
    ]);
    expect(without[0].summary).toBe("");
    expect(without[0].summaryTruncated).toBe(false);
  });

  it("marks a StopFailure close as failed, with the CLI's reason", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "do it" } }),
      ev({
        seq: 2,
        ts: 13,
        kind: "turn_end",
        turnId: "T1",
        payload: {
          failed: true,
          error: "rate_limit",
          errorDetails: "Retry after 60s",
          filesChanged: [{ status: "M", path: "a.ts" }],
        },
      }),
    ]);
    expect(turns[0].failed).toBe(true);
    expect(turns[0].error).toBe("rate_limit");
    expect(turns[0].errorDetails).toBe("Retry after 60s");
    // The failure still closes the turn and keeps its diff.
    expect(turns[0].endTs).toBe(13);
    expect(turns[0].files).toEqual([{ status: "M", path: "a.ts" }]);

    // A normal close stays unmarked.
    const ok = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T2", payload: { prompt: "do it" } }),
      ev({ seq: 2, ts: 13, kind: "turn_end", turnId: "T2", payload: { postSha: "abc" } }),
    ]);
    expect(ok[0].failed).toBe(false);
    expect(ok[0].error).toBeUndefined();
  });

  it("marks an Interrupt close as cut short — closed, with its diff, but not failed", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "do it" } }),
      ev({
        seq: 2,
        ts: 13,
        kind: "turn_end",
        turnId: "T1",
        payload: { interrupted: true, filesChanged: [{ status: "M", path: "a.ts" }] },
      }),
    ]);
    expect(turns[0].interrupted).toBe(true);
    expect(turns[0].failed).toBe(false);
    expect(turns[0].endTs).toBe(13);
    expect(turns[0].files).toEqual([{ status: "M", path: "a.ts" }]);

    // A normal close, and a StopFailure close, stay unmarked.
    const ok = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T2", payload: { prompt: "do it" } }),
      ev({ seq: 2, ts: 13, kind: "turn_end", turnId: "T2", payload: { failed: true } }),
    ]);
    expect(ok[0].interrupted).toBe(false);
  });

  it("a new prompt on the same terminal ends the turn still open there (CHAT-2026-10 F2)", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", termId: "t", payload: { prompt: "a" } }),
      ev({ seq: 2, ts: 20, kind: "prompt", turnId: "T2", termId: "t", payload: { prompt: "b" } }),
      // Another terminal's open turn is not touched.
      ev({ seq: 3, ts: 30, kind: "prompt", turnId: "T3", termId: "u", payload: { prompt: "c" } }),
    ]);
    expect(turns.map((t) => [t.turnId, t.endTs, t.interrupted, t.superseded])).toEqual([
      ["T1", 20, true, true],
      ["T2", null, false, false],
      ["T3", null, false, false],
    ]);
    // The Rust side's synthetic close carries the same marks.
    const closed = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", termId: "t", payload: { prompt: "a" } }),
      ev({
        seq: 2,
        ts: 20,
        kind: "turn_end",
        turnId: "T1",
        termId: "t",
        payload: { interrupted: true, superseded: true },
      }),
      ev({ seq: 3, ts: 20, kind: "prompt", turnId: "T2", termId: "t", payload: { prompt: "b" } }),
    ]);
    expect(closed[0].superseded).toBe(true);
    expect(closed[0].endTs).toBe(20);
    expect(closed[1].endTs).toBeNull();
  });

  it("carries the rewind bindings: term, session, cwd and post-turn snapshot", () => {
    const turns = buildTimeline([
      ev({
        seq: 1,
        ts: 10,
        kind: "prompt",
        turnId: "T1",
        termId: "term-1",
        sessionId: "sid-1",
        payload: { prompt: "do it", cwd: "/repo/worktree" },
      }),
      ev({
        seq: 2,
        ts: 13,
        kind: "turn_end",
        turnId: "T1",
        payload: { postSha: "abc123" },
      }),
    ]);
    expect(turns[0].termId).toBe("term-1");
    expect(turns[0].sessionId).toBe("sid-1");
    expect(turns[0].cwd).toBe("/repo/worktree");
    expect(turns[0].postSha).toBe("abc123");
  });

  it("a rewind event marks the turn it points at, and never opens a phantom turn", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "do it" } }),
      ev({ seq: 2, ts: 13, kind: "turn_end", turnId: "T1", payload: { postSha: "abc" } }),
      ev({
        seq: 3,
        ts: 20,
        kind: "rewind",
        turnId: "T1",
        payload: { restoredSha: "abc", forkSessionId: "fork-1" },
      }),
      // Rewind pointing at a turn this slice of the ledger no longer holds.
      ev({ seq: 4, ts: 21, kind: "rewind", turnId: "GONE", payload: {} }),
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0].rewound).toBe(true);
  });

  it("collects check runs and commits inside their turn (T4b)", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "fix" } }),
      ev({
        seq: 2,
        ts: 11,
        kind: "tool_result",
        turnId: "T1",
        payload: { tool: "Bash", command: "cargo test" },
      }),
      ev({
        seq: 3,
        ts: 11,
        kind: "check_run",
        turnId: "T1",
        payload: { command: "cargo test", status: "failed", exitCode: 101, durationMs: 4200 },
      }),
      ev({
        seq: 4,
        ts: 12,
        kind: "check_run",
        turnId: "T1",
        payload: { command: "cargo test", status: "passed" },
      }),
      ev({ seq: 5, ts: 13, kind: "turn_end", turnId: "T1", payload: {} }),
      ev({
        seq: 6,
        ts: 14,
        kind: "commit",
        turnId: "T1",
        actor: "human",
        payload: { sha: "deadbeef1234", subject: "Fix it", files: ["a.rs", "b.rs"], amend: false },
      }),
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0].checks).toEqual([
      { command: "cargo test", status: "failed", exitCode: 101, durationMs: 4200 },
      { command: "cargo test", status: "passed", exitCode: undefined, durationMs: undefined },
    ]);
    expect(turns[0].commits).toEqual([
      { sha: "deadbeef1234", subject: "Fix it", files: 2, amend: false },
    ]);
    // A tool_result still counts as a tool call; the check line is extra.
    expect(turns[0].toolResults).toBe(1);
  });

  it("carries the turn's verification verdict with its gates and live claims (F1c)", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "fix" } }),
      ev({
        seq: 2,
        ts: 12,
        kind: "turn_end",
        turnId: "T1",
        payload: { summary: "All tests pass." },
      }),
      ev({
        seq: 3,
        ts: 12,
        kind: "verification",
        turnId: "T1",
        actor: "console",
        payload: {
          status: "unverified_claim",
          reason: '"all tests pass" but no check ran in this turn',
          coverage: "per_tool",
          checks: { passed: 0, failed: 0, empty: 0, unknown: 0, interrupted: 0 },
          claims: [
            { phrase: "all tests pass", lang: "en", negated: false },
            { phrase: "lint passes", lang: "en", negated: true },
          ],
          gates: { test: { command: "npm test", status: "not_run" } },
        },
      }),
    ]);
    expect(turns[0].verification).toEqual({
      seq: 3,
      status: "unverified_claim",
      reason: '"all tests pass" but no check ran in this turn',
      coverage: "per_tool",
      checks: { passed: 0, failed: 0, empty: 0, unknown: 0, interrupted: 0 },
      claims: ["all tests pass"],
      gates: { test: "not_run" },
      gateCommands: { test: "npm test" },
    });
    // F1d: the human's answer rides on the same turn.
    const decided = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "fix" } }),
      ev({
        seq: 2,
        ts: 12,
        kind: "turn_end",
        turnId: "T1",
        payload: { summary: "All tests pass." },
      }),
      ev({
        seq: 3,
        ts: 12,
        kind: "verification",
        turnId: "T1",
        actor: "console",
        payload: { status: "claim_contradicted", reason: "x", coverage: "per_tool" },
      }),
      ev({
        seq: 4,
        ts: 13,
        kind: "verification_decision",
        turnId: "T1",
        actor: "human",
        payload: { decision: "accept", reason: "flaky suite, ran it by hand", verificationSeq: 3 },
      }),
    ]);
    expect(decided[0].verification?.decision).toEqual({
      decision: "accept",
      reason: "flaky suite, ran it by hand",
    });
  });

  it("reads what a turn cost from its close: Claude's delta, OpenCode's sum, Codex's tokens (F2)", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", termId: "a", payload: { prompt: "x" } }),
      ev({
        seq: 2,
        ts: 11,
        kind: "turn_end",
        turnId: "T1",
        termId: "a",
        payload: {
          usage: {
            engine: "claude",
            source: "statusline",
            costUsd: 1.25,
            costDeltaUsd: 0.25,
            model: "claude-opus-5-5",
            tokens: { context: 42000, output: 800 },
            quota: { fiveHourPct: 23.5, sevenDayPct: 41.2 },
          },
        },
      }),
      ev({ seq: 3, ts: 12, kind: "prompt", turnId: "T2", termId: "b", payload: { prompt: "y" } }),
      ev({
        seq: 4,
        ts: 13,
        kind: "turn_end",
        turnId: "T2",
        termId: "b",
        payload: {
          usage: {
            engine: "opencode",
            source: "plugin",
            costUsd: 0.002,
            tokens: { input: 803, output: 74 },
          },
        },
      }),
      ev({ seq: 5, ts: 14, kind: "prompt", turnId: "T3", termId: "c", payload: { prompt: "z" } }),
      ev({
        seq: 6,
        ts: 15,
        kind: "turn_end",
        turnId: "T3",
        termId: "c",
        payload: {
          usage: {
            engine: "codex",
            source: "rollout",
            tokens: { input: 480, output: 41, cacheRead: 12800 },
          },
        },
      }),
    ]);
    expect(turns[0].usage).toEqual({
      engine: "claude",
      source: "statusline",
      costUsd: 1.25,
      costTurnUsd: 0.25,
      tokensIn: undefined,
      tokensOut: 800,
      cacheRead: undefined,
      model: "claude-opus-5-5",
      quota: { fiveHourPct: 23.5, sevenDayPct: 41.2, spendPct: undefined },
    });
    expect(turns[1].usage?.costTurnUsd).toBe(0.002);
    expect(turns[1].usage?.tokensIn).toBe(803);
    expect(turns[2].usage?.costTurnUsd).toBeUndefined();
    expect(turns[2].usage?.tokensOut).toBe(41);
    expect(turns[2].usage?.cacheRead).toBe(12800);
  });

  it("keeps a check's kind and its honest verdict: empty and unknown are not passed (F1a)", () => {
    const turns = buildTimeline([
      ev({ seq: 1, ts: 10, kind: "prompt", turnId: "T1", payload: { prompt: "fix" } }),
      ev({
        seq: 2,
        ts: 11,
        kind: "check_run",
        turnId: "T1",
        payload: { command: "npx vitest run", kind: "test", status: "empty", exitCode: 0 },
      }),
      ev({
        seq: 3,
        ts: 12,
        kind: "check_run",
        turnId: "T1",
        payload: { command: "npm run lint", kind: "lint" },
      }),
      ev({ seq: 4, ts: 13, kind: "turn_end", turnId: "T1", payload: {} }),
    ]);
    expect(turns[0].checks.map((c) => [c.kind, c.status])).toEqual([
      ["test", "empty"],
      ["lint", "unknown"],
    ]);
  });

  it("lists tool calls, injected docs, model and case per turn (T5)", () => {
    const turns = buildTimeline([
      ev({
        seq: 1,
        ts: 10,
        kind: "prompt",
        turnId: "T1",
        caseId: "jira:X-1",
        payload: { prompt: "go" },
      }),
      ev({
        seq: 2,
        ts: 10,
        kind: "context_injected",
        turnId: "T1",
        payload: { docs: ["memory:a.md", "skill:b"] },
      }),
      ev({
        seq: 3,
        ts: 11,
        kind: "session_start",
        turnId: "T1",
        payload: { model: "claude-opus-5" },
      }),
      ev({
        seq: 4,
        ts: 12,
        kind: "tool_result",
        turnId: "T1",
        payload: { tool: "Bash", command: "cargo test", excerpt: "ok", agentId: "sub-1" },
      }),
      ev({
        seq: 5,
        ts: 13,
        kind: "tool_result",
        turnId: "T1",
        payload: {
          tool: "Bash",
          command: "npm test",
          excerpt: "Exit code 1",
          failed: true,
          exitCode: 1,
        },
      }),
      ev({
        seq: 6,
        ts: 14,
        kind: "tool_result",
        turnId: "T1",
        payload: { tool: "Read", excerpt: "line 1" },
      }),
      ev({
        seq: 7,
        ts: 15,
        kind: "model_switch",
        turnId: "T1",
        payload: { to: "claude-haiku-4-5" },
      }),
    ]);
    const t = turns[0];
    expect(t.caseId).toBe("jira:X-1");
    expect(t.injected).toEqual(["memory:a.md", "skill:b"]);
    expect(t.model).toBe("claude-haiku-4-5");
    expect(t.toolResults).toBe(3);
    expect(t.tools).toEqual([
      {
        tool: "Bash",
        command: "cargo test",
        excerpt: "ok",
        failed: false,
        exitCode: undefined,
        agentId: "sub-1",
      },
      {
        tool: "Bash",
        command: "npm test",
        excerpt: "Exit code 1",
        failed: true,
        exitCode: 1,
        agentId: undefined,
      },
      {
        tool: "Read",
        command: undefined,
        excerpt: "line 1",
        failed: false,
        exitCode: undefined,
        agentId: undefined,
      },
    ]);
  });

  it("skips turnless events (case_link, job_run)", () => {
    const turns = buildTimeline([
      ev({ seq: 0, ts: 1, kind: "case_link", actor: "system" }),
      ev({ seq: 1, ts: 2, kind: "job_run", actor: "system" }),
    ]);
    expect(turns).toEqual([]);
  });
});

function session(partial: Partial<TerminalSession>): TerminalSession {
  return {
    id: "term-1",
    name: "shell 1",
    cwd: "/repo",
    createdAtMs: 0,
    initialScrollback: "",
    liveScrollback: "",
    status: "stopped",
    agent: "claude",
    ...partial,
  };
}

function turn(partial: Partial<TimelineTurn>): TimelineTurn {
  return {
    turnId: "T1",
    ts: 10,
    prompt: "do it",
    approvals: [],
    toolResults: 0,
    files: [],
    filesTruncated: false,
    endTs: 13,
    summary: "",
    summaryTruncated: false,
    failed: false,
    interrupted: false,
    superseded: false,
    rewound: false,
    checks: [],
    commits: [],
    tools: [],
    injected: [],
    termId: "term-1",
    sessionId: "sid-original",
    cwd: "/repo/worktree",
    postSha: "post-sha",
    ...partial,
  };
}

describe("rewindToTurn", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProofStore.setState({ projectRoot: null });
    useTerminalsStore.setState({
      projectRoot: "/repo",
      sessions: [session({})],
      activeId: null,
      ready: true,
    });
  });

  it("restores in the turn's checkout and opens a new session bound to the fork", async () => {
    mockRewind.mockResolvedValueOnce({
      backupSha: "backup-sha",
      forkSessionId: "fork-uuid",
      forkError: null,
    });

    await useProofStore.getState().rewindToTurn(turn({}));

    // The turn's cwd (its worktree), never the session's current cwd.
    expect(mockRewind).toHaveBeenCalledWith(
      expect.objectContaining({
        repo: "/repo/worktree",
        commitSha: "post-sha",
        sessionId: "sid-original",
        cutoffMs: 13,
        termId: "term-1",
        turnId: "T1",
      }),
    );
    expect(refresh).toHaveBeenCalled();
    // A NEW session exists, bound to the fork BEFORE its terminal spawns —
    // that binding is what makes it launch `--resume <fork>`.
    const sessions = useTerminalsStore.getState().sessions;
    expect(sessions).toHaveLength(2);
    const forked = sessions[1];
    expect(forked.agentSessionId).toBe("fork-uuid");
    expect(forked.cwd).toBe("/repo/worktree");
    expect(forked.agent).toBe("claude");
    expect(showToast).toHaveBeenCalledWith(expect.stringMatching(/rewound/i), "success");
  });

  it("degrades honestly: fork failed ⇒ files restored, NO new session, loud toast", async () => {
    mockRewind.mockResolvedValueOnce({
      backupSha: "backup-sha",
      forkSessionId: null,
      forkError: "claude 2.1.100 predates 2.1.224",
    });

    await useProofStore.getState().rewindToTurn(turn({}));

    expect(useTerminalsStore.getState().sessions).toHaveLength(1);
    expect(showToast).toHaveBeenCalledWith(expect.stringMatching(/NOT rewound/), "error");
  });

  it("refuses on a live source session and on engines without transcript fork", async () => {
    useTerminalsStore.setState({
      sessions: [session({ status: "live" })],
    });
    await useProofStore.getState().rewindToTurn(turn({}));
    expect(mockRewind).not.toHaveBeenCalled();

    useTerminalsStore.setState({
      sessions: [session({ agent: "codex" })],
    });
    await useProofStore.getState().rewindToTurn(turn({}));
    expect(mockRewind).not.toHaveBeenCalled();

    // Session gone entirely — the engine is unknowable, so no rewind.
    useTerminalsStore.setState({ sessions: [] });
    await useProofStore.getState().rewindToTurn(turn({}));
    expect(mockRewind).not.toHaveBeenCalled();
  });

  it("refuses a turn that is still open or has no post-turn snapshot", async () => {
    await useProofStore.getState().rewindToTurn(turn({ endTs: null }));
    await useProofStore.getState().rewindToTurn(turn({ postSha: undefined }));
    await useProofStore.getState().rewindToTurn(turn({ sessionId: undefined }));
    expect(mockRewind).not.toHaveBeenCalled();
  });
});

describe("following the active session's ledger", () => {
  beforeEach(() => {
    useProofStore.getState().clear();
    vi.mocked(ipc.testigoList).mockReset().mockResolvedValue([]);
    vi.mocked(ipc.ledgerRootFor).mockReset();
  });

  it("loads the ledger the backend resolves for the session's folder", async () => {
    vi.mocked(ipc.ledgerRootFor).mockResolvedValue("/work/backend");
    await useProofStore.getState().followCheckout("/work/backend/sub");
    expect(ipc.ledgerRootFor).toHaveBeenCalledWith("/work/backend/sub");
    expect(useProofStore.getState().projectRoot).toBe("/work/backend");
    expect(ipc.testigoList).toHaveBeenLastCalledWith("/work/backend");
  });

  it("a slow read of the previous ledger never overwrites the new one", async () => {
    let releaseOld: (v: ProofEvent[]) => void = () => {};
    vi.mocked(ipc.testigoList)
      .mockImplementationOnce(() => new Promise((r) => (releaseOld = r)))
      .mockResolvedValueOnce([ev({ seq: 7, caseId: "term:new" })]);
    const old = useProofStore.getState().load("/work/central");
    await useProofStore.getState().load("/work/backend");
    releaseOld([ev({ seq: 1, caseId: "term:old" })]);
    await old;
    expect(useProofStore.getState().projectRoot).toBe("/work/backend");
    expect(useProofStore.getState().events.map((e) => e.caseId)).toEqual(["term:new"]);
  });

  it("switching ledgers drops the open case; reloading the same one keeps it", async () => {
    await useProofStore.getState().load("/work/central");
    useProofStore.getState().selectCase("term:a");
    await useProofStore.getState().load("/work/central");
    expect(useProofStore.getState().selectedCase).toBe("term:a");
    await useProofStore.getState().load("/work/backend");
    expect(useProofStore.getState().selectedCase).toBeNull();
  });

  it("only the latest follow wins when resolutions arrive out of order", async () => {
    let releaseFirst: (v: string) => void = () => {};
    vi.mocked(ipc.ledgerRootFor)
      .mockImplementationOnce(() => new Promise((r) => (releaseFirst = r)))
      .mockResolvedValueOnce("/work/second");
    const first = useProofStore.getState().followCheckout("/work/first");
    await useProofStore.getState().followCheckout("/work/second");
    releaseFirst("/work/first");
    await first;
    expect(useProofStore.getState().projectRoot).toBe("/work/second");
  });
});

describe("exportAat (D3: IETF audit trail)", () => {
  beforeEach(() => {
    useProofStore.setState({ projectRoot: "/p", lastAat: null, exporting: null, error: null });
    vi.mocked(ipc.testigoExportAat).mockReset();
    vi.mocked(ipc.testigoVerifyAat).mockReset();
  });

  it("exports, then re-verifies every chain it wrote, and keeps both", async () => {
    const summary = { dir: "/p/proofpacks/aat/c1", sessions: 1, records: 3, files: [], draft: "d" };
    vi.mocked(ipc.testigoExportAat).mockResolvedValue(summary);
    vi.mocked(ipc.testigoVerifyAat).mockResolvedValue([
      [
        "/p/proofpacks/aat/c1/s.jsonl",
        { ok: true, records: 3, brokenAt: null, tailClosed: true, problems: [] },
      ],
    ]);
    await useProofStore.getState().exportAat("c1");
    expect(ipc.testigoExportAat).toHaveBeenCalledWith("/p", "c1");
    expect(ipc.testigoVerifyAat).toHaveBeenCalledWith("/p/proofpacks/aat/c1");
    const s = useProofStore.getState();
    expect(s.lastAat?.summary).toEqual(summary);
    expect(s.lastAat?.reports[0][1].ok).toBe(true);
    expect(s.exporting).toBeNull();
  });

  it("surfaces a failed export and clears the busy flag", async () => {
    vi.mocked(ipc.testigoExportAat).mockRejectedValue(new Error("Nothing to export"));
    await useProofStore.getState().exportAat();
    const s = useProofStore.getState();
    expect(s.error).toContain("Nothing to export");
    expect(s.exporting).toBeNull();
    expect(s.lastAat).toBeNull();
  });
});
