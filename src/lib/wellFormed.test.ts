import { describe, expect, it } from "vitest";
import { tailWellFormed, toWellFormed } from "./wellFormed";

/// What serde_json does on the other side of the IPC: a lone surrogate in
/// the JSON text is a hard parse error. JSON.stringify escapes a lone
/// surrogate as \udXXX, so "no lone escape left" is the same check.
const survivesIpc = (s: string) =>
  !/\\ud[89ab][0-9a-f]{2}(?!\\ud[c-f][0-9a-f]{2})|(?<!\\ud[89ab][0-9a-f]{2})\\ud[c-f][0-9a-f]{2}/i.test(
    JSON.stringify(s),
  );

describe("toWellFormed", () => {
  it("returns plain text untouched", () => {
    const s = "npm test\r\n✓ 14 passed";
    expect(toWellFormed(s)).toBe(s);
  });

  it("keeps valid pairs", () => {
    expect(toWellFormed("ok 🚀 done")).toBe("ok 🚀 done");
  });

  it("replaces a lone high and a lone low surrogate", () => {
    expect(toWellFormed("a\uD83Db")).toBe("a�b");
    expect(toWellFormed("a\uDE80b")).toBe("a�b");
    expect(toWellFormed("\uDE80\uD83D")).toBe("��");
  });
});

describe("tailWellFormed", () => {
  it("returns a short string as is", () => {
    expect(tailWellFormed("abc", 10)).toBe("abc");
  });

  it("cuts by code unit when the boundary is clean", () => {
    expect(tailWellFormed("abcdef", 3)).toBe("def");
  });

  it("never starts inside a pair (the #138 case)", () => {
    // "🚀" is two code units; a cut of 3 from the tail lands on its low half.
    const s = "x🚀ab";
    expect(s.slice(s.length - 3).charCodeAt(0)).toBe(0xde80);
    const out = tailWellFormed(s, 3);
    expect(out).toBe("ab");
    expect(survivesIpc(out)).toBe(true);
  });

  it("holds for every cut point of an emoji-heavy scrollback", () => {
    const s = "line 🚀 one\r\n✅ ok 👩‍💻 done\r\n".repeat(20);
    for (let max = 1; max <= s.length; max++) {
      const out = tailWellFormed(s, max);
      expect(out.length).toBeLessThanOrEqual(max);
      expect(out.length).toBeGreaterThanOrEqual(Math.min(max, s.length) - 1);
      expect(survivesIpc(out)).toBe(true);
      expect(s.endsWith(out)).toBe(true);
    }
  });

  it("repairs a surrogate that was already lone before the cut", () => {
    const out = tailWellFormed("aaaa\uD83Dbbbb", 6);
    expect(out).toBe("a�bbbb");
  });
});
