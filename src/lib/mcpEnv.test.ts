import { describe, expect, it } from "vitest";
import { looksLikeGithubMcp, setEnvLine } from "./mcpEnv";

describe("setEnvLine", () => {
  it("fills an empty box, appends to others, and replaces an existing key", () => {
    expect(setEnvLine("", "GITHUB_TOKEN", "t1")).toBe("GITHUB_TOKEN=t1");
    expect(setEnvLine("A=1", "GITHUB_TOKEN", "t1")).toBe("A=1\nGITHUB_TOKEN=t1");
    expect(setEnvLine("A=1\nGITHUB_TOKEN=old\nB=2", "GITHUB_TOKEN", "new")).toBe(
      "A=1\nGITHUB_TOKEN=new\nB=2",
    );
    expect(setEnvLine("  GITHUB_TOKEN=ghp_…", "GITHUB_TOKEN", "new")).toBe("GITHUB_TOKEN=new");
  });
  it("recognises a GitHub MCP command", () => {
    expect(looksLikeGithubMcp("npx -y @modelcontextprotocol/server-github")).toBe(true);
    expect(looksLikeGithubMcp("docker run ghcr.io/github/github-mcp-server")).toBe(true);
    expect(looksLikeGithubMcp("npx -y @modelcontextprotocol/server-filesystem")).toBe(false);
  });
});
