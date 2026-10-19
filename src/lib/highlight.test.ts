import { describe, expect, it } from "vitest";
import { tokenize } from "./highlight";

describe("tokenize", () => {
  it("marks comments, strings, numbers and keywords; the rest stays plain", () => {
    const t = tokenize('const a = "x"; // note\nreturn 42;', "ts");
    expect(t).toEqual([
      { kind: "keyword", text: "const" },
      { kind: "plain", text: " a = " },
      { kind: "string", text: '"x"' },
      { kind: "plain", text: "; " },
      { kind: "comment", text: "// note" },
      { kind: "plain", text: "\n" },
      { kind: "keyword", text: "return" },
      { kind: "plain", text: " " },
      { kind: "number", text: "42" },
      { kind: "plain", text: ";" },
    ]);
  });
  it("picks the comment family from the fence language", () => {
    expect(tokenize("# hi\nx = 1", "py")[0]).toEqual({ kind: "comment", text: "# hi" });
    // In TypeScript a '#' is not a comment (private fields, strings…).
    expect(tokenize("a.#b", "ts").every((t) => t.kind !== "comment")).toBe(true);
  });
  it("round-trips the input text exactly", () => {
    const src = "fn main() {\n  let s = 'it\\'s';\n  /* multi\n line */ 3.14\n}";
    expect(
      tokenize(src, "rs")
        .map((t) => t.text)
        .join(""),
    ).toBe(src);
    expect(tokenize("").length).toBe(0);
  });
});
