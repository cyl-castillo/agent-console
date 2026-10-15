import { describe, expect, it, vi } from "vitest";

vi.mock("../ipc/tauri", () => ({ ipc: { sponsorGet: vi.fn(), sponsorSet: vi.fn() } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { shouldAutoApprove, weekSpend, WEEK_MS } from "./sponsorStore";
import type { Sponsor } from "../types/domain";

const sponsor = (over: Partial<Sponsor> = {}): Sponsor => ({
  objective: "ship",
  owner: "Carlos",
  weeklyBudgetUsd: 20,
  riskThreshold: "low",
  autoApprove: true,
  ...over,
});

describe("sponsor policy (D11)", () => {
  it("approves alone only within the threshold and the budget", () => {
    expect(shouldAutoApprove(sponsor(), "low", 5).yes).toBe(true);
    expect(shouldAutoApprove(sponsor(), "medium", 5).yes).toBe(false);
    expect(shouldAutoApprove(sponsor({ riskThreshold: "medium" }), "medium", 5).yes).toBe(true);
    expect(shouldAutoApprove(sponsor({ riskThreshold: "medium" }), "high", 5).yes).toBe(false);
    expect(shouldAutoApprove(sponsor(), "low", 20).why).toBe("weekly budget reached");
    expect(shouldAutoApprove(sponsor({ weeklyBudgetUsd: 0 }), "low", 999).yes).toBe(true);
    expect(shouldAutoApprove(sponsor({ autoApprove: false }), "low", 0).yes).toBe(false);
    expect(shouldAutoApprove(null, "low", 0).yes).toBe(false);
  });

  it("sums this week's sessions only", () => {
    const now = 10 * WEEK_MS;
    const sessions = [
      { id: "a", createdAtMs: now - 1000 },
      { id: "b", createdAtMs: now - 3 * 24 * 3600 * 1000 },
      { id: "old", createdAtMs: now - WEEK_MS - 1 },
    ];
    const byTerm = { a: { costUsd: 1.25 }, b: { costUsd: 2.5 }, old: { costUsd: 100 } };
    expect(weekSpend(sessions, byTerm, now)).toBe(3.75);
    expect(weekSpend(sessions, {}, now)).toBe(0);
  });
});
