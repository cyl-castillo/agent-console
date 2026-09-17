import { describe as suite, expect, it } from "vitest";

import {
  describe,
  diffLines,
  editPreview,
  MAX_EDIT_DISTANCE,
  type DiffLine,
} from "./approvalPreview";
import type { ApprovalRequest } from "../types/domain";

const render = (lines: DiffLine[]) =>
  lines.map((l) => (l.type === "add" ? "+" : l.type === "del" ? "-" : " ") + l.text);

const stat = (lines: DiffLine[]) => ({
  add: lines.filter((l) => l.type === "add").length,
  del: lines.filter((l) => l.type === "del").length,
  ctx: lines.filter((l) => l.type === "ctx").length,
});

function req(tool: string, input: Record<string, unknown>): ApprovalRequest {
  return { id: "r1", tool, input, cwd: "/repo" } as unknown as ApprovalRequest;
}

suite("diffLines", () => {
  it("shows unchanged lines as context and keeps the stat honest", () => {
    const old = ["a", "b", "c", "d"].join("\n");
    const neu = ["a", "B", "c", "d"].join("\n");
    expect(render(diffLines(old, neu))).toEqual([" a", "-b", "+B", " c", " d"]);
    expect(stat(diffLines(old, neu))).toEqual({ add: 1, del: 1, ctx: 3 });
  });

  it("handles pure insertions and deletions without a phantom empty line", () => {
    expect(diffLines("", "")).toEqual([]);
    expect(render(diffLines("", "x\ny"))).toEqual(["+x", "+y"]);
    expect(render(diffLines("x\ny", ""))).toEqual(["-x", "-y"]);
  });

  it("aligns insertions in the middle and at the edges", () => {
    expect(render(diffLines("a\nc", "a\nb\nc"))).toEqual([" a", "+b", " c"]);
    expect(render(diffLines("b\nc", "a\nb\nc"))).toEqual(["+a", " b", " c"]);
    expect(render(diffLines("a\nb", "a\nb\nc"))).toEqual([" a", " b", "+c"]);
    expect(render(diffLines("a\nb\nc", "a\nc"))).toEqual([" a", "-b", " c"]);
  });

  it("finds a minimal script when lines move around", () => {
    const d = diffLines("a\nb\nc\nd\ne", "a\nc\nb\ne\nf");
    // 5 unchanged-ish lines, two edits worth of churn — never a full rewrite.
    const s = stat(d);
    expect(s.ctx).toBeGreaterThanOrEqual(3);
    expect(s.add + s.del).toBeLessThanOrEqual(4);
    // And the script replays to the new text.
    const replay = d.filter((l) => l.type !== "del").map((l) => l.text);
    expect(replay).toEqual(["a", "c", "b", "e", "f"]);
    const back = d.filter((l) => l.type !== "add").map((l) => l.text);
    expect(back).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("stays fast on a large block with a small change (the modal's worst case)", () => {
    const lines = Array.from({ length: 6000 }, (_, i) => `line ${i}: ${"x".repeat(20)}`);
    const changed = [...lines];
    changed[3000] = "line 3000: CHANGED";
    changed.splice(4500, 0, "inserted");
    const t0 = performance.now();
    const d = diffLines(lines.join("\n"), changed.join("\n"));
    const ms = performance.now() - t0;
    expect(stat(d)).toEqual({ add: 2, del: 1, ctx: 5999 });
    expect(ms).toBeLessThan(500);
  });

  it("gives up on alignment past the edit cap and shows del+add, still bounded", () => {
    const a = Array.from({ length: 3000 }, (_, i) => `old ${i}`).join("\n");
    const b = Array.from({ length: 3000 }, (_, i) => `new ${i}`).join("\n");
    const t0 = performance.now();
    const d = diffLines(a, b);
    expect(performance.now() - t0).toBeLessThan(2000);
    expect(stat(d)).toEqual({ add: 3000, del: 3000, ctx: 0 });
    expect(3000 + 3000).toBeGreaterThan(MAX_EDIT_DISTANCE);
  });

  it("keeps shared prefix and suffix as context around a capped middle", () => {
    const pre = ["same 1", "same 2"];
    const post = ["same end"];
    const a = [...pre, ...Array.from({ length: 800 }, (_, i) => `o${i}`), ...post].join("\n");
    const b = [...pre, ...Array.from({ length: 800 }, (_, i) => `n${i}`), ...post].join("\n");
    const d = diffLines(a, b);
    expect(render(d).slice(0, 2)).toEqual([" same 1", " same 2"]);
    expect(render(d)[render(d).length - 1]).toBe(" same end");
    expect(stat(d)).toEqual({ add: 800, del: 800, ctx: 3 });
  });
});

suite("editPreview", () => {
  it("diffs Edit/StrReplace and lists Write as additions", () => {
    expect(render(editPreview(req("Edit", { old_string: "a\nb", new_string: "a\nc" }))!)).toEqual([
      " a",
      "-b",
      "+c",
    ]);
    expect(render(editPreview(req("Write", { content: "x\ny" }))!)).toEqual(["+x", "+y"]);
    expect(editPreview(req("Edit", {}))).toBeNull();
    expect(editPreview(req("Write", {}))).toBeNull();
    expect(editPreview(req("Bash", { command: "ls" }))).toBeNull();
  });

  it("separates MultiEdit hunks and returns null when there is nothing", () => {
    const d = editPreview(
      req("MultiEdit", {
        edits: [
          { old_string: "a", new_string: "b" },
          { old_string: "c", new_string: "d" },
        ],
      }),
    )!;
    expect(render(d)).toEqual(["-a", "+b", " ···", "-c", "+d"]);
    expect(editPreview(req("MultiEdit", { edits: [] }))).toBeNull();
  });
});

suite("describe", () => {
  it("names the command, the file, or the tool", () => {
    expect(describe(req("Bash", { command: "npm test", description: "run tests" }))).toEqual({
      primary: "npm test",
      secondary: "run tests",
    });
    expect(describe(req("Bash", {}))).toEqual({ primary: "(no command)", secondary: undefined });
    expect(describe(req("Edit", { file_path: "/repo/src/a.ts" }))).toEqual({
      primary: "src/a.ts",
      secondary: "Edit",
    });
    const other = describe(req("WebFetch", { url: "https://x" }));
    expect(other.secondary).toBe("WebFetch");
    expect(other.primary).toContain("https://x");
  });
});
