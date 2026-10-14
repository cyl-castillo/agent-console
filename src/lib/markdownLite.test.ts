import { describe, expect, it } from "vitest";
import { parseMarkdownLite } from "./markdownLite";

describe("parseMarkdownLite", () => {
  it("keeps raw HTML as literal text — the model never renders markup", () => {
    const blocks = parseMarkdownLite('Done. <img src=x onerror="alert(1)"> and <b>bold</b>');
    expect(blocks).toEqual([
      {
        kind: "paragraph",
        inlines: [{ kind: "text", text: 'Done. <img src=x onerror="alert(1)"> and <b>bold</b>' }],
      },
    ]);
  });
  it("parses the summary subset: headings, bullets, fences, inline code", () => {
    const blocks = parseMarkdownLite(
      "## What changed\n- fixed `token` expiry\n- added **tests**\n\n```ts\nconst a = 1;\n```\nAll green.",
    );
    expect(blocks.map((b) => b.kind)).toEqual(["heading", "list", "code", "paragraph"]);
    const list = blocks[1];
    if (list.kind !== "list") throw new Error("expected list");
    expect(list.items[0]).toEqual([
      { kind: "text", text: "fixed " },
      { kind: "code", text: "token" },
      { kind: "text", text: " expiry" },
    ]);
    const code = blocks[2];
    if (code.kind !== "code") throw new Error("expected code");
    expect(code.lang).toBe("ts");
    expect(code.text).toBe("const a = 1;");
  });
  it("joins wrapped lines into one paragraph and splits on blank lines", () => {
    const blocks = parseMarkdownLite("one\ntwo\n\nthree");
    expect(blocks).toHaveLength(2);
  });
});
