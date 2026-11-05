import { describe, expect, it } from "vitest";
import { gapSentence, waitLabel } from "./decisionGap";

describe("decision gap (MOVIL F0)", () => {
  it("labels waits coarsely: seconds, minutes, hours", () => {
    expect(waitLabel(1_800)).toBe("2 s");
    expect(waitLabel(90_000)).toBe("2 min");
    expect(waitLabel(7 * 60_000)).toBe("7 min");
    expect(waitLabel(110_520_000)).toBe("31 h");
  });

  it("says nothing without requests, and says 'all answered here' when nothing was missed", () => {
    expect(gapSentence(undefined)).toBeNull();
    expect(
      gapSentence({ windowDays: 7, requests: 0, decided: 0, pending: 0, unseen: 0 }),
    ).toBeNull();
    expect(
      gapSentence({
        windowDays: 7,
        requests: 3,
        decided: 3,
        pending: 0,
        unseen: 0,
        p90WaitMs: 2_000,
        maxWaitMs: 9_000,
      }),
    ).toBe(
      "Last 7 days: all 3 decisions were answered here. The ones taken here waited 2 s at the 90th percentile and 9 s at most.",
    );
  });

  it("names the unseen count against the total and skips the waits when the console took none", () => {
    expect(gapSentence({ windowDays: 7, requests: 10, decided: 0, pending: 1, unseen: 9 })).toBe(
      "Last 7 days: 9 of 10 decisions timed out or were answered in the CLI before this screen saw them. A paired device catches them wherever you are.",
    );
  });
});
