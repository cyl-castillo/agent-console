import { describe, expect, it } from "vitest";

import { summarizeCases, buildTimeline } from "./proofStore";
import type { ProofEvent } from "../types/domain";

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

  it("skips turnless events (case_link, job_run)", () => {
    const turns = buildTimeline([
      ev({ seq: 0, ts: 1, kind: "case_link", actor: "system" }),
      ev({ seq: 1, ts: 2, kind: "job_run", actor: "system" }),
    ]);
    expect(turns).toEqual([]);
  });
});
