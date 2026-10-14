import { describe, expect, it } from "vitest";
import { tierFor } from "./decisions";
import type { ApprovalRequest } from "../types/domain";

const req = (tool: string, input: Record<string, unknown>): ApprovalRequest => ({
  id: "x",
  ts: 0,
  sessionDir: "/home/u/.claude",
  cwd: "/home/u/proj",
  tool,
  input,
});

describe("tierFor", () => {
  it("read-only commands are low", () => {
    expect(tierFor(req("Bash", { command: "git status" })).tier).toBe("low");
    expect(tierFor(req("Bash", { command: "npm test -- --run x" })).tier).toBe("low");
    expect(tierFor(req("Bash", { command: "find . -name '*.ts'" })).tier).toBe("low");
  });
  it("unknown commands are medium", () => {
    expect(tierFor(req("Bash", { command: "node scripts/migrate.js" })).tier).toBe("medium");
  });
  it("irreversible or external commands are high", () => {
    expect(tierFor(req("Bash", { command: "rm -rf dist" })).tier).toBe("high");
    expect(tierFor(req("Bash", { command: "git push origin main" })).tier).toBe("high");
    expect(tierFor(req("Bash", { command: "find . -delete" })).tier).toBe("high");
  });
  it("edits inside the project are medium, outside or secrets are high", () => {
    expect(tierFor(req("Edit", { file_path: "/home/u/proj/src/a.ts" })).tier).toBe("medium");
    expect(tierFor(req("Edit", { file_path: "/home/u/proj/tests/a.test.ts" })).label).toBe(
      "Edits a test",
    );
    expect(tierFor(req("Write", { file_path: "/etc/hosts" })).tier).toBe("high");
    expect(tierFor(req("Write", { file_path: "/home/u/proj/.env" })).tier).toBe("high");
  });
  it("reads are low, tool servers medium", () => {
    expect(tierFor(req("Read", { file_path: "/x" })).tier).toBe("low");
    expect(tierFor(req("mcp__github__create_issue", {})).tier).toBe("medium");
  });
});
