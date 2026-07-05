import { describe, expect, it, vi } from "vitest";

// The module imports Tauri APIs at load time — isolate them so the pure URL
// builder can be tested without a webview.
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import { buildIssueUrl } from "./reportProblem";

describe("buildIssueUrl (prefilled GitHub issue)", () => {
  it("targets the repo's new-issue page with bug labels", () => {
    const url = new URL(buildIssueUrl({ version: "1.2.3", userAgent: "UA" }));
    expect(url.origin + url.pathname).toBe(
      "https://github.com/cyl-castillo/agent-console/issues/new",
    );
    expect(url.searchParams.get("labels")).toBe("bug,field-report");
  });

  it("embeds version and platform in the body", () => {
    const url = new URL(buildIssueUrl({ version: "1.2.3", userAgent: "Linux-UA" }));
    const body = url.searchParams.get("body") ?? "";
    expect(body).toContain("v1.2.3");
    expect(body).toContain("Linux-UA");
  });

  it("prefills the error text in the title and as a code block in the body", () => {
    const url = new URL(
      buildIssueUrl({ version: "1.2.3", userAgent: "UA", error: "Copy failed: denied" }),
    );
    expect(url.searchParams.get("title")).toBe("[bug] Copy failed: denied");
    expect(url.searchParams.get("body")).toContain("```\nCopy failed: denied\n```");
  });

  it("truncates very long errors in the title, keeping the body intact", () => {
    const long = "x".repeat(200);
    const url = new URL(buildIssueUrl({ version: "1", userAgent: "UA", error: long }));
    expect(url.searchParams.get("title")!.length).toBeLessThanOrEqual("[bug] ".length + 80);
    expect(url.searchParams.get("body")).toContain(long);
  });

  it("handles a missing version gracefully", () => {
    const url = new URL(buildIssueUrl({ version: "", userAgent: "UA" }));
    expect(url.searchParams.get("body")).toContain("vunknown");
  });
});
