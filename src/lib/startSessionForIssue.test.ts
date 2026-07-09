import { describe, expect, it } from "vitest";

import { seedForIssue } from "./startSessionForIssue";
import type { JiraIssue } from "../types/domain";

function issue(over: Partial<JiraIssue> = {}): JiraIssue {
  return {
    key: "ABC-123",
    summary: "Fix the login redirect",
    status: "To Do",
    statusCategory: "new",
    priority: "High",
    issueType: "Bug",
    dueDate: null,
    project: "Acme",
    updated: null,
    url: "https://acme.atlassian.net/browse/ABC-123",
    ...over,
  };
}

describe("seedForIssue (the prompt the agent session starts with)", () => {
  it("includes the key, summary, and a browse ref", () => {
    const s = seedForIssue(issue());
    expect(s).toContain("ABC-123");
    expect(s).toContain("Fix the login redirect");
    expect(s).toContain("https://acme.atlassian.net/browse/ABC-123");
  });

  it("folds type and priority into a parenthetical when present", () => {
    expect(seedForIssue(issue())).toContain("(Bug, High priority)");
  });

  it("omits the parenthetical cleanly when type and priority are absent", () => {
    const s = seedForIssue(issue({ issueType: "", priority: null }));
    expect(s).not.toContain("()");
    expect(s).toContain("ABC-123: Fix the login redirect. Ref:");
  });
});
