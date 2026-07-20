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

  it("skips turnless events (case_link, job_run)", () => {
    const turns = buildTimeline([
      ev({ seq: 0, ts: 1, kind: "case_link", actor: "system" }),
      ev({ seq: 1, ts: 2, kind: "job_run", actor: "system" }),
    ]);
    expect(turns).toEqual([]);
  });
});
