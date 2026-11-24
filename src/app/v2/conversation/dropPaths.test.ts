import { describe, expect, it } from "vitest";
import { draftWithDropped, quotePath } from "./dropPaths";

describe("dropped paths", () => {
  it("quotes a path, spaces included", () => {
    expect(quotePath("/home/me/My Docs/a.png")).toBe('"/home/me/My Docs/a.png"');
  });

  it("falls back to single quotes when the path holds a double quote", () => {
    expect(quotePath('/tmp/a"b.txt')).toBe(`'/tmp/a"b.txt'`);
  });

  it("starts an empty draft with the paths and a trailing space", () => {
    expect(draftWithDropped("", ["/a", "/b c"])).toBe('"/a" "/b c" ');
  });

  it("appends after what was already typed", () => {
    expect(draftWithDropped("look at", ["/a"])).toBe('look at "/a" ');
    expect(draftWithDropped("look at ", ["/a"])).toBe('look at "/a" ');
    expect(draftWithDropped("line\n", ["/a"])).toBe('line\n"/a" ');
  });

  it("leaves the draft alone when nothing was dropped", () => {
    expect(draftWithDropped("hi", [])).toBe("hi");
    expect(draftWithDropped("hi", [""])).toBe("hi");
  });
});
