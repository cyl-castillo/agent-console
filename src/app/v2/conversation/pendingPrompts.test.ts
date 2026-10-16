import { describe, expect, it } from "vitest";
import { UNWITNESSED_GRACE_MS, WITNESS_SLACK_MS, holdsQueue, unwitnessed } from "./pendingPrompts";

describe("pendingPrompts (the composer's echo)", () => {
  const t0 = 1_700_000_000_000;

  it("a turn witnesses the echo only when recorded after the send", () => {
    const pending = [{ text: "ok", ts: t0 }];
    // An older turn with the same text does not claim it.
    expect(unwitnessed(pending, [{ prompt: "ok", ts: t0 - 60_000 }])).toEqual(pending);
    // The turn the send produced does (the hook records it a beat later).
    expect(unwitnessed(pending, [{ prompt: "ok ", ts: t0 + 300 }])).toEqual([]);
    // Within the clock slack still counts.
    expect(unwitnessed(pending, [{ prompt: "ok", ts: t0 - WITNESS_SLACK_MS + 1 }])).toEqual([]);
    // A different prompt never does.
    expect(unwitnessed(pending, [{ prompt: "no", ts: t0 + 300 }])).toEqual(pending);
  });

  it("holds the queue while a send is recent, lets go after the grace", () => {
    const pending = [{ text: "x", ts: t0 }];
    expect(holdsQueue(pending, t0 + 1_000)).toBe(true);
    expect(holdsQueue(pending, t0 + UNWITNESSED_GRACE_MS)).toBe(false);
    expect(holdsQueue([], t0)).toBe(false);
  });
});
