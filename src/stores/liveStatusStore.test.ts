import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { freshStatus, LIVE_STATUS_FRESH_MS, useLiveStatusStore } from "./liveStatusStore";

beforeEach(() => {
  useLiveStatusStore.setState({ byTerm: {} });
});

describe("live status (statusLine renders)", () => {
  it("keeps the latest render per terminal and ignores unbound ones", () => {
    const st = useLiveStatusStore.getState();
    st.note({ ts: 1, termId: "a", costUsd: 0.1 });
    st.note({ ts: 2, termId: "a", costUsd: 0.2 });
    st.note({ ts: 3, costUsd: 9 });
    expect(useLiveStatusStore.getState().byTerm.a.costUsd).toBe(0.2);
    expect(Object.keys(useLiveStatusStore.getState().byTerm)).toEqual(["a"]);
  });

  it("freshStatus hides a render older than the freshness window", () => {
    const byTerm = { a: { ts: 1_000_000, termId: "a", contextUsed: 10 } };
    expect(freshStatus(byTerm, "a", 1_000_000 + 5_000)?.contextUsed).toBe(10);
    expect(freshStatus(byTerm, "a", 1_000_000 + LIVE_STATUS_FRESH_MS + 1)).toBeNull();
    expect(freshStatus(byTerm, "missing", 1_000_000)).toBeNull();
  });
});
