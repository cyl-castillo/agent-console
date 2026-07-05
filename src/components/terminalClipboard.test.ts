import { describe, expect, it } from "vitest";

import { clipboardActionFor, type KeyLike } from "./terminalClipboard";

function key(over: Partial<KeyLike> = {}): KeyLike {
  return {
    type: "keydown",
    key: "c",
    ctrlKey: true,
    metaKey: false,
    shiftKey: false,
    ...over,
  };
}

describe("clipboardActionFor (terminal copy/paste policy)", () => {
  it("Ctrl+Shift+C with a selection copies (keeps the selection)", () => {
    expect(clipboardActionFor(key({ key: "C", shiftKey: true }), true)).toBe("copy");
  });

  it("plain Ctrl+C with a selection copies and clears it", () => {
    expect(clipboardActionFor(key(), true)).toBe("copy-and-clear");
  });

  it("plain Ctrl+C without a selection passes through — SIGINT is sacred", () => {
    expect(clipboardActionFor(key(), false)).toBeNull();
  });

  it("Ctrl+Shift+C without a selection passes through (nothing to copy)", () => {
    expect(clipboardActionFor(key({ key: "C", shiftKey: true }), false)).toBeNull();
  });

  it("Ctrl+Shift+V pastes", () => {
    expect(clipboardActionFor(key({ key: "V", shiftKey: true }), true)).toBe("paste");
    expect(clipboardActionFor(key({ key: "V", shiftKey: true }), false)).toBe("paste");
  });

  it("plain Ctrl+V passes through (the native paste event flow handles it)", () => {
    expect(clipboardActionFor(key({ key: "v" }), false)).toBeNull();
  });

  it("Cmd works as the modifier too (macOS)", () => {
    expect(
      clipboardActionFor(key({ ctrlKey: false, metaKey: true }), true),
    ).toBe("copy-and-clear");
  });

  it("ignores keyup and unmodified keys", () => {
    expect(clipboardActionFor(key({ type: "keyup" }), true)).toBeNull();
    expect(clipboardActionFor(key({ ctrlKey: false }), true)).toBeNull();
    expect(clipboardActionFor(key({ key: "x" }), true)).toBeNull();
  });
});
