import { describe, expect, it } from "vitest";
import { sessionAttention, type AttentionInputs } from "./attention";

const base = (over: Partial<AttentionInputs> = {}): AttentionInputs => ({
  session: { id: "a", status: "live" },
  approvalTermIds: [],
  waitingTermId: null,
  workingUntil: 0,
  workingTermId: null,
  activeId: "a",
  now: 1000,
  ...over,
});

describe("sessionAttention", () => {
  it("a stopped session is stopped whatever else is pending", () => {
    expect(
      sessionAttention(base({ session: { id: "a", status: "stopped" }, approvalTermIds: ["a"] })),
    ).toBe("stopped");
  });
  it("a pending approval on the session means it needs you", () => {
    expect(sessionAttention(base({ approvalTermIds: ["a"], workingUntil: 5000 }))).toBe(
      "needs-you",
    );
  });
  it("the CLI waiting at its own prompt means it needs you", () => {
    expect(sessionAttention(base({ waitingTermId: "a" }))).toBe("needs-you");
    expect(sessionAttention(base({ waitingTermId: "b" }))).toBe("done");
  });
  it("the working window belongs to the terminal it names, else the active one", () => {
    expect(sessionAttention(base({ workingUntil: 5000, workingTermId: "b" }))).toBe("done");
    expect(sessionAttention(base({ workingUntil: 5000, workingTermId: "a" }))).toBe("working");
    expect(sessionAttention(base({ workingUntil: 5000, activeId: "a" }))).toBe("working");
    expect(sessionAttention(base({ workingUntil: 5000, activeId: "b" }))).toBe("done");
  });
  it("an expired working window is idle", () => {
    expect(sessionAttention(base({ workingUntil: 500 }))).toBe("done");
  });
});
