import { describe, expect, it } from "vitest";
import type { TimelineTurn, TimelineVerification } from "../stores/proofStore";
import { evidencePrompt, pendingVerifications, verificationStats } from "./verificationInbox";

const base = (ts: number): TimelineTurn => ({
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
});
const verification = (
  status: string,
  extra: Partial<TimelineVerification> = {},
): TimelineVerification => ({
  seq: 1,
  status,
  reason: "r",
  coverage: "per_tool",
  checks: { passed: 0, failed: 0, empty: 0, unknown: 0, interrupted: 0 },
  claims: [],
  gates: {},
  gateCommands: {},
  ...extra,
});

describe("verification inbox (F1d)", () => {
  it("lists only undecided unverified, stale or contradicted turns without a review", () => {
    const turns: TimelineTurn[] = [
      { ...base(1), verification: verification("unverified_claim") },
      { ...base(2), verification: verification("claim_contradicted") },
      { ...base(3), verification: verification("stale_check") },
      { ...base(4), verification: verification("verified") },
      { ...base(5), verification: verification("checked") },
      { ...base(6), verification: verification("unwitnessed") },
      {
        ...base(7),
        verification: verification("unverified_claim", { decision: { decision: "accept" } }),
      },
      {
        ...base(8),
        verification: verification("unverified_claim"),
        review: { verdict: "request_changes", findings: 1, reviewer: "agent:reviewer:codex" },
      },
      base(9),
    ];
    expect(pendingVerifications(turns).map((t) => t.turnId)).toEqual(["t1", "t2", "t3"]);
  });

  it("asks for the gate the project names and the turn did not run", () => {
    const t = {
      ...base(1),
      verification: verification("unverified_claim", {
        gates: { test: "not_run", lint: "passed" },
        gateCommands: { test: "pnpm test", lint: "pnpm run lint" },
      }),
    };
    expect(evidencePrompt(t)).toContain("Run `pnpm test` now");
    expect(evidencePrompt(t)).toContain("exit code");
    const unknown = { ...base(2), verification: verification("stale_check") };
    expect(evidencePrompt(unknown)).toContain("project's test suite");
  });

  it("counts the week's habit: claims backed by a passing check after the last edit", () => {
    const now = 100 * 86_400_000;
    const turns: TimelineTurn[] = [
      { ...base(now - 1000), verification: verification("verified") },
      { ...base(now - 2000), verification: verification("unverified_claim") },
      { ...base(now - 3000), verification: verification("claim_contradicted") },
      { ...base(now - 4000), verification: verification("checked") },
      { ...base(now - 10 * 86_400_000), verification: verification("verified") }, // outside the window
    ];
    expect(verificationStats(turns, now)).toEqual({
      turns: 4,
      claims: 3,
      verified: 1,
      unverified: 1,
      contradicted: 1,
    });
  });
});
