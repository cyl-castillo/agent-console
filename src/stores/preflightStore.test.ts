import { describe, expect, it } from "vitest";

import type { Preflight } from "../types/domain";
import { enginesFound, toolStatus } from "./preflightStore";

const tool = (name: string, found: boolean) => ({
  name,
  found,
  version: found ? "1.0.0" : null,
  fixCommand: null,
  fixNote: null,
});
const preflight = (found: string[]): Preflight => ({
  tools: ["claude", "codex", "opencode", "git", "node"].map((n) => tool(n, found.includes(n))),
  auth: [],
});

describe("enginesFound", () => {
  it("is empty before the probe answers and when no agent CLI is installed", () => {
    expect(enginesFound(null)).toEqual([]);
    expect(enginesFound(preflight(["git", "node"]))).toEqual([]);
  });

  it("counts any one of the three engines, not only Claude", () => {
    expect(enginesFound(preflight(["opencode", "git"]))).toEqual(["opencode"]);
    expect(enginesFound(preflight(["codex"]))).toEqual(["codex"]);
    expect(enginesFound(preflight(["claude"]))).toEqual(["claude"]);
  });

  it("lists every engine found, in a stable order, and never a non-engine tool", () => {
    expect(enginesFound(preflight(["node", "opencode", "git", "claude"]))).toEqual([
      "claude",
      "opencode",
    ]);
  });

  it("treats a result from an older backend without an opencode row as not found", () => {
    const old: Preflight = { tools: [tool("claude", false), tool("codex", true)], auth: [] };
    expect(toolStatus(old, "opencode")).toBeUndefined();
    expect(enginesFound(old)).toEqual(["codex"]);
  });
});
