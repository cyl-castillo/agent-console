import { describe, expect, it } from "vitest";
import type { TimelineTurn } from "../stores/proofStore";
import { fmtUsd, sessionLedgerCost, usageWeek } from "./usageStats";

const base = (ts: number, termId: string): TimelineTurn => ({
  turnId: `t${ts}`,
  ts,
  prompt: "p",
  approvals: [],
  toolResults: 0,
  files: [],
  filesTruncated: false,
  endTs: ts + 1,
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
  termId,
});

describe("usage stats (F2)", () => {
  it("sums the week by engine and counts unpriced (Codex) turns apart", () => {
    const now = 100 * 86_400_000;
    const turns: TimelineTurn[] = [
      {
        ...base(now - 1000, "a"),
        usage: {
          engine: "claude",
          source: "statusline",
          costUsd: 1.5,
          costTurnUsd: 0.5,
          tokensOut: 100,
        },
      },
      {
        ...base(now - 2000, "a"),
        usage: {
          engine: "claude",
          source: "statusline",
          costUsd: 1.0,
          costTurnUsd: 1.0,
          tokensOut: 300,
        },
      },
      {
        ...base(now - 3000, "b"),
        usage: {
          engine: "opencode",
          source: "plugin",
          costUsd: 0.02,
          costTurnUsd: 0.02,
          tokensOut: 40,
        },
      },
      { ...base(now - 4000, "c"), usage: { engine: "codex", source: "rollout", tokensOut: 900 } },
      {
        ...base(now - 10 * 86_400_000, "a"),
        usage: { engine: "claude", source: "statusline", costTurnUsd: 9 },
      },
      base(now - 5000, "d"),
    ];
    expect(usageWeek(turns, now)).toEqual({
      turns: 4,
      costUsd: 1.52,
      byEngine: {
        claude: { turns: 2, costUsd: 1.5, tokensOut: 400 },
        opencode: { turns: 1, costUsd: 0.02, tokensOut: 40 },
        codex: { turns: 1, costUsd: 0, tokensOut: 900 },
      },
      unpricedTurns: 1,
    });
    expect(sessionLedgerCost(turns, "a")).toBe(10.5);
    expect(sessionLedgerCost(turns, "c")).toBeUndefined();
    expect(fmtUsd(0.004)).toBe("<$0.01");
    expect(fmtUsd(1.2345)).toBe("$1.23");
  });
});
