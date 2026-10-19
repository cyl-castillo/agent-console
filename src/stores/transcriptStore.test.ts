import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import {
  TRANSCRIPT_CAP,
  itemsFor,
  useTranscriptStore,
  type TranscriptItem,
} from "./transcriptStore";

const text = (id: string, ts = 1): TranscriptItem => ({ kind: "text", id, ts, text: id });

describe("transcriptStore", () => {
  beforeEach(() => useTranscriptStore.setState({ bySession: {} }));

  it("appends deltas per session, dedupes by kind+id, and a backfill replaces", () => {
    const { apply } = useTranscriptStore.getState();
    apply({ sessionId: "s1", engine: "claude", backfill: false, items: [text("a"), text("b")] });
    apply({ sessionId: "s1", engine: "claude", backfill: false, items: [text("b"), text("c")] });
    apply({ sessionId: "s2", engine: "codex", backfill: false, items: [text("z")] });
    expect(itemsFor("s1")(useTranscriptStore.getState()).map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect(itemsFor("s2")(useTranscriptStore.getState()).map((i) => i.id)).toEqual(["z"]);
    apply({ sessionId: "s1", engine: "claude", backfill: true, items: [text("q")] });
    expect(itemsFor("s1")(useTranscriptStore.getState()).map((i) => i.id)).toEqual(["q"]);
  });

  it("returns the same empty array for an unknown session and caps the history", () => {
    const s = useTranscriptStore.getState();
    expect(itemsFor("nope")(s)).toBe(itemsFor(undefined)(s));
    const many = Array.from({ length: TRANSCRIPT_CAP + 50 }, (_, i) => text(`i${i}`));
    s.apply({ sessionId: "s1", engine: "claude", backfill: false, items: many });
    const kept = itemsFor("s1")(useTranscriptStore.getState());
    expect(kept.length).toBe(TRANSCRIPT_CAP);
    expect(kept[0].id).toBe("i50");
    const before = useTranscriptStore.getState();
    before.apply({ sessionId: "s1", engine: "claude", backfill: false, items: [text("i60")] });
    expect(useTranscriptStore.getState()).toBe(before);
    before.clear("s1");
    expect(itemsFor("s1")(useTranscriptStore.getState()).length).toBe(0);
  });
});
