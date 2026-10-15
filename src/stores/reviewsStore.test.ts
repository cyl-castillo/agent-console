import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { useReviewsStore } from "./reviewsStore";
import type { ReviewVerdict } from "../types/domain";

const verdict = (over: Partial<ReviewVerdict> = {}): ReviewVerdict => ({
  verdict: "block",
  findings: [],
  reviewer: "reviewer:codex",
  reviewerEngine: "codex",
  termId: "t1",
  turnId: "turn-1",
  caseId: "c1",
  files: 2,
  tokens: 1,
  costUsd: 0,
  rationale: "",
  seq: 10,
  ...over,
});

describe("reviewsStore (D10)", () => {
  beforeEach(() => useReviewsStore.setState({ items: [] }));

  it("keeps one decision per verdict and resolves it", () => {
    const s = useReviewsStore.getState();
    s.add("/p", verdict(), 1);
    s.add("/p", verdict(), 2); // the same ledger entry again: no duplicate
    expect(useReviewsStore.getState().items).toHaveLength(1);
    s.add("/p", verdict({ seq: 11 }), 3);
    expect(useReviewsStore.getState().items).toHaveLength(2);
    const id = useReviewsStore.getState().items[0].id;
    s.resolve(id);
    expect(useReviewsStore.getState().items.find((x) => x.id === id)?.resolved).toBe(true);
    expect(useReviewsStore.getState().items[1].resolved).toBe(false);
  });
});
