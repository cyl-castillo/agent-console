import { describe, expect, it } from "vitest";

import { summarizeCases } from "./proofStore";
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
