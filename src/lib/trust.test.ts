import { describe, expect, it } from "vitest";
import type { TrustStatus } from "../types/domain";
import { surfaceSummary, trustAllowsLaunch, trustNeedsDecision, trustTier } from "./trust";

const status = (
  state: string,
  counts: Partial<TrustStatus["surface"]["counts"]> = {},
): TrustStatus => ({
  state,
  decision: null,
  surface: {
    files: [],
    digest: "d",
    executes: Boolean(
      (counts.hooks ?? 0) + (counts.plugins ?? 0) + (counts.setup ?? 0) + (counts.mcp ?? 0),
    ),
    counts: {
      hooks: 0,
      mcp: 0,
      plugins: 0,
      rules: 0,
      instructions: 0,
      skills: 0,
      setup: 0,
      ...counts,
    },
  },
});

describe("workspace trust (F4)", () => {
  it("asks when undecided or changed, launches only when trusted", () => {
    expect(trustNeedsDecision(status("undecided"))).toBe(true);
    expect(trustNeedsDecision(status("changed"))).toBe(true);
    expect(trustNeedsDecision(status("trusted"))).toBe(false);
    expect(trustNeedsDecision(status("declined"))).toBe(false);
    expect(trustNeedsDecision(null)).toBe(false);
    expect(trustAllowsLaunch(status("trusted"))).toBe(true);
    expect(trustAllowsLaunch(status("declined"))).toBe(false);
    expect(trustAllowsLaunch(undefined)).toBe(false);
  });

  it("names what would run and ranks executing files higher", () => {
    expect(surfaceSummary(status("undecided"))).toBe("nothing that shapes the agents");
    expect(surfaceSummary(status("undecided", { hooks: 2, instructions: 1, mcp: 1 }))).toBe(
      "2 hook files · 1 MCP config · 1 instructions file",
    );
    expect(trustTier(status("undecided", { instructions: 1, skills: 3 }))).toBe("medium");
    expect(trustTier(status("undecided", { plugins: 1 }))).toBe("high");
  });
});
