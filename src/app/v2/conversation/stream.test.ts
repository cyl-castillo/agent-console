import { describe, expect, it } from "vitest";
import type { TranscriptItem } from "../../../stores/transcriptStore";
import { CLOCK_SLACK_MS, EMPTY_STREAM, saidText, streamFor } from "./stream";

const T0 = 1_000_000;
const items: TranscriptItem[] = [
  { kind: "user", id: "p1", ts: T0 - 60_000, text: "earlier prompt" },
  { kind: "text", id: "a0", ts: T0 - 50_000, text: "old reply" },
  { kind: "user", id: "p2", ts: T0 + 200, text: "fix the build" },
  { kind: "tool_use", id: "t1", ts: T0 + 1000, name: "Bash", input: "npm test" },
  { kind: "tool_result", id: "t1", ts: T0 + 3000, ok: true },
  { kind: "text", id: "a1", ts: T0 + 3500, text: "Tests pass." },
  { kind: "tool_use", id: "t2", ts: T0 + 4000, name: "Edit", input: "src/a.ts" },
];

describe("streamFor", () => {
  it("takes the items after the turn's prompt and reports the running tool while live", () => {
    const s = streamFor(items, { ts: T0, endTs: null });
    expect(s.texts).toEqual([{ id: "a1", text: "Tests pass." }]);
    expect(s.toolUses).toBe(2);
    expect(s.running).toEqual({ name: "Edit", input: "src/a.ts" });
  });
  it("closes the window at endTs (with slack) and nothing runs in a closed turn", () => {
    const s = streamFor(items, { ts: T0, endTs: T0 + 4000 });
    expect(s.toolUses).toBe(2);
    expect(s.running).toBeNull();
    const earlier = streamFor(items, { ts: T0 - 60_000 + 100, endTs: T0 - 10_000 });
    expect(earlier.texts.map((t) => t.text)).toEqual(["old reply"]);
    expect(earlier.toolUses).toBe(0);
  });
  it("stops at the next prompt even inside a live window and is empty outside", () => {
    const s = streamFor(items, { ts: T0 - 60_000 + 100, endTs: null });
    expect(s.texts.map((t) => t.text)).toEqual(["old reply"]);
    expect(streamFor(items, { ts: T0 + 100_000, endTs: null })).toBe(EMPTY_STREAM);
    expect(streamFor([], { ts: T0, endTs: null })).toBe(EMPTY_STREAM);
    // A beat of clock skew either way still lands in the turn.
    expect(streamFor(items, { ts: T0 + 200 + CLOCK_SLACK_MS - 1, endTs: null }).texts.length).toBe(
      1,
    );
  });
  it("saidText prefers the stream and falls back to the summary", () => {
    expect(
      saidText(
        {
          texts: [
            { id: "a", text: "x" },
            { id: "b", text: "y" },
          ],
          toolUses: 0,
          running: null,
        },
        "s",
      ),
    ).toBe("x\n\ny");
    expect(saidText(EMPTY_STREAM, "summary")).toBe("summary");
  });
});
