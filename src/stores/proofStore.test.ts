import { beforeEach, describe, expect, it, vi } from "vitest";

// rewindToTurn rewrites a working tree and spawns a session resuming a forked
// conversation — isolate the backend and the stores it touches.
vi.mock("../ipc/tauri", () => ({
  ipc: {
    turnRewind: vi.fn(),
    testigoList: vi.fn().mockResolvedValue([]),
    testigoVerify: vi.fn().mockResolvedValue(null),
    testigoGetSettings: vi.fn().mockResolvedValue(null),
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
    rewound: false,
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
