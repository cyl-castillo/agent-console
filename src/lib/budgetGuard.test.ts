import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/tauri", () => ({
  ipc: { testigoBudgetEvent: vi.fn() },
}));

import { ipc } from "../ipc/tauri";
import { findBreaches, recordBudget } from "./budgetGuard";
import type { TerminalSession } from "../stores/terminalsStore";

function session(id: string, status: TerminalSession["status"] = "live"): TerminalSession {
  return { id, name: id, status } as TerminalSession;
}

describe("findBreaches (D5 hard stop)", () => {
  const budgets = { caps: { a: 10, b: 5 }, defaultCap: 0, breached: {} as Record<string, number> };

  it("reports live, capped sessions whose cost reached the cap, once", () => {
    const byTerm = { a: { costUsd: 10 }, b: { costUsd: 4.99 }, c: { costUsd: 100 } };
    const out = findBreaches(byTerm, budgets, [session("a"), session("b"), session("c")]);
    expect(out.map((b) => b.session.id)).toEqual(["a"]);
    expect(out[0]).toMatchObject({ spent: 10, cap: 10 });
    // Already marked: not reported again.
    expect(findBreaches(byTerm, { ...budgets, breached: { a: 10 } }, [session("a")])).toEqual([]);
  });

  it("ignores sessions that are not live, uncapped, or without a reported cost", () => {
    const byTerm = { a: { costUsd: 50 }, d: {} };
    expect(findBreaches(byTerm, budgets, [session("a", "stopped")])).toEqual([]);
    expect(findBreaches(byTerm, budgets, [session("d")])).toEqual([]);
    expect(findBreaches({}, budgets, [session("a")])).toEqual([]);
  });

  it("applies the project default cap when a session has none", () => {
    const out = findBreaches({ z: { costUsd: 3 } }, { ...budgets, defaultCap: 2 }, [session("z")]);
    expect(out).toHaveLength(1);
    expect(out[0].cap).toBe(2);
  });
});

describe("recordBudget (D5 evidence)", () => {
  it("witnesses budget_reached and budget_decision with the amounts", async () => {
    vi.mocked(ipc.testigoBudgetEvent)
      .mockClear()
      .mockResolvedValue({} as never);
    const s = { id: "t1", agentSessionId: "s1" };
    expect(await recordBudget("/p", s, 12.3, 10)).toBe(true);
    expect(ipc.testigoBudgetEvent).toHaveBeenCalledWith(
      "/p",
      "t1",
      "s1",
      12.3,
      10,
      undefined,
      undefined,
    );
    expect(await recordBudget("/p", s, 12.3, 10, "raise", 15)).toBe(true);
    expect(ipc.testigoBudgetEvent).toHaveBeenLastCalledWith(
      "/p",
      "t1",
      "s1",
      12.3,
      10,
      "raise",
      15,
    );
  });

  it("does nothing without a project root", async () => {
    vi.mocked(ipc.testigoBudgetEvent).mockClear();
    expect(await recordBudget(null, { id: "t1" }, 1, 1)).toBe(false);
    expect(ipc.testigoBudgetEvent).not.toHaveBeenCalled();
  });

  it("is best-effort: a refusing ledger (witnessing off) never throws", async () => {
    vi.mocked(ipc.testigoBudgetEvent).mockRejectedValue(new Error("witnessing disabled"));
    let r: unknown = "unset";
    try {
      r = await recordBudget("/p", { id: "t1" }, 1, 1, "stop");
    } catch (e) {
      r = `threw ${String(e)}`;
    }
    expect(r).toBe(false);
  });
});
