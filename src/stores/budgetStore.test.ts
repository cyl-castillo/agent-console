import { beforeEach, describe, expect, it } from "vitest";
import { useBudgetStore } from "./budgetStore";

/// F10: a hand-over keeps the breach (so the hard stop does not fire again
/// on the idle session) and marks the session; raising the cap clears both.
describe("budgetStore hand-over marks", () => {
  beforeEach(() => {
    useBudgetStore.setState({
      caps: {},
      defaultCap: 0,
      breached: {},
      handedOff: {},
      quotaDismissed: [],
    });
  });

  it("marks a handed-off session without clearing its breach", () => {
    const s = useBudgetStore.getState();
    s.setCap("a", 5);
    s.markBreached("a", 5.2);
    s.markHandedOff("a");
    const st = useBudgetStore.getState();
    expect(st.breached.a).toBe(5.2);
    expect(st.handedOff.a).toBeGreaterThan(0);
  });

  it("raising the cap above the breach clears the mark; clearing the breach does too", () => {
    const s = useBudgetStore.getState();
    s.setCap("a", 5);
    s.markBreached("a", 5.2);
    s.markHandedOff("a");
    s.setCap("a", 5.1);
    expect(useBudgetStore.getState().handedOff.a).toBeGreaterThan(0);
    s.setCap("a", 10);
    expect(useBudgetStore.getState().handedOff.a).toBeUndefined();
    expect(useBudgetStore.getState().breached.a).toBeUndefined();
    s.markBreached("b", 1);
    s.markHandedOff("b");
    s.clearBreach("b");
    expect(useBudgetStore.getState().handedOff.b).toBeUndefined();
  });

  it("remembers dismissed quota turns, deduplicated and bounded", () => {
    const s = useBudgetStore.getState();
    s.dismissQuota("t1");
    s.dismissQuota("t1");
    expect(useBudgetStore.getState().quotaDismissed).toEqual(["t1"]);
    for (let i = 0; i < 60; i++) s.dismissQuota(`x${i}`);
    const list = useBudgetStore.getState().quotaDismissed;
    expect(list.length).toBe(50);
    expect(list).not.toContain("t1");
    expect(list[list.length - 1]).toBe("x59");
  });
});
