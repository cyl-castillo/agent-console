import { beforeEach, describe, expect, it } from "vitest";

import { confirmDialog, useConfirmStore } from "./confirmStore";

beforeEach(() => {
  useConfirmStore.setState({ pending: null, queue: [] });
});

describe("confirmDialog", () => {
  it("shows the request and resolves with the answer", async () => {
    const p = confirmDialog("Delete it?");
    const pending = useConfirmStore.getState().pending;
    expect(pending?.message).toBe("Delete it?");
    useConfirmStore.getState().settle(true);
    await expect(p).resolves.toBe(true);
    expect(useConfirmStore.getState().pending).toBeNull();
  });

  it("resolves false on cancel and carries the request options", async () => {
    const p = confirmDialog({ message: "m", title: "t", danger: true, confirmLabel: "Delete" });
    const pending = useConfirmStore.getState().pending;
    expect(pending?.title).toBe("t");
    expect(pending?.danger).toBe(true);
    expect(pending?.confirmLabel).toBe("Delete");
    useConfirmStore.getState().settle(false);
    await expect(p).resolves.toBe(false);
  });

  it("queues a second ask instead of replacing the first", async () => {
    const first = confirmDialog("first");
    const second = confirmDialog("second");
    expect(useConfirmStore.getState().pending?.message).toBe("first");
    expect(useConfirmStore.getState().queue).toHaveLength(1);
    useConfirmStore.getState().settle(true);
    await expect(first).resolves.toBe(true);
    expect(useConfirmStore.getState().pending?.message).toBe("second");
    useConfirmStore.getState().settle(false);
    await expect(second).resolves.toBe(false);
    expect(useConfirmStore.getState().queue).toHaveLength(0);
  });

  it("settle with nothing pending is a no-op", () => {
    expect(() => useConfirmStore.getState().settle(true)).not.toThrow();
    expect(useConfirmStore.getState().pending).toBeNull();
  });
});
