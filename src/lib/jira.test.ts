import { describe, expect, it } from "vitest";

import {
  intentForIssue,
  seedForIssue,
  groupIssuesByStatus,
  formatSecondsForWorklog,
  priorityLevel,
  dueState,
  typeDotClass,
} from "./jira";
import type { JiraIssue } from "../types/domain";

function issue(over: Partial<JiraIssue> = {}): JiraIssue {
  return {
    key: "ABC-1",
    summary: "Fix the login redirect",
    status: "To Do",
    statusCategory: "new",
    priority: "High",
    issueType: "Story",
    dueDate: null,
    project: "Acme",
    updated: null,
    url: "https://acme.atlassian.net/browse/ABC-1",
    ...over,
  };
}

describe("intentForIssue", () => {
  it("a fresh (To Do) story is 'implement'", () => {
    expect(intentForIssue(issue())).toBe("implement");
  });
  it("any status containing 'review' is 'review' (the reported bug)", () => {
    expect(intentForIssue(issue({ status: "Code Review", statusCategory: "indeterminate" }))).toBe(
      "review",
    );
    expect(intentForIssue(issue({ status: "In Review", statusCategory: "indeterminate" }))).toBe(
      "review",
    );
    // Review wins even for a bug — you review the fix, not re-debug it.
    expect(intentForIssue(issue({ status: "Peer Review", issueType: "Bug" }))).toBe("review");
  });
  it("testing/QA statuses are 'test'", () => {
    expect(intentForIssue(issue({ status: "QA", statusCategory: "indeterminate" }))).toBe("test");
    expect(intentForIssue(issue({ status: "Testing", statusCategory: "indeterminate" }))).toBe(
      "test",
    );
  });
  it("a bug (not in review/test) is 'debug' regardless of stage", () => {
    expect(intentForIssue(issue({ issueType: "Bug", statusCategory: "new" }))).toBe("debug");
    expect(
      intentForIssue(
        issue({ issueType: "Defect", status: "In Progress", statusCategory: "indeterminate" }),
      ),
    ).toBe("debug");
  });
  it("a non-bug already in progress is 'continue'", () => {
    expect(intentForIssue(issue({ status: "In Progress", statusCategory: "indeterminate" }))).toBe(
      "continue",
    );
  });
});

describe("seedForIssue", () => {
  it("a review ticket asks for a review, not an implementation", () => {
    const s = seedForIssue(issue({ status: "Code Review", statusCategory: "indeterminate" }));
    expect(s).toMatch(/review/i);
    expect(s).not.toMatch(/plan and implement/i);
    expect(s).toContain("ABC-1");
    expect(s).toContain("https://acme.atlassian.net/browse/ABC-1");
  });
  it("a fresh story asks to plan and implement", () => {
    expect(seedForIssue(issue())).toMatch(/plan and implement/i);
  });
  it("a bug asks to reproduce and diagnose before fixing", () => {
    expect(seedForIssue(issue({ issueType: "Bug" }))).toMatch(/reproduce and diagnose/i);
  });
  it("always carries the key and browse ref", () => {
    for (const status of ["To Do", "In Progress", "Code Review", "QA"]) {
      const s = seedForIssue(issue({ status }));
      expect(s).toContain("ABC-1");
      expect(s).toContain("browse/ABC-1");
    }
  });
});

describe("groupIssuesByStatus", () => {
  it("groups by status and orders new before indeterminate", () => {
    const groups = groupIssuesByStatus([
      issue({ key: "A", status: "In Review", statusCategory: "indeterminate" }),
      issue({ key: "B", status: "To Do", statusCategory: "new" }),
      issue({ key: "C", status: "To Do", statusCategory: "new" }),
    ]);
    expect(groups.map((g) => g.status)).toEqual(["To Do", "In Review"]);
    expect(groups[0].issues.map((i) => i.key)).toEqual(["B", "C"]);
  });
  it("preserves incoming (due-date) order within a group", () => {
    const groups = groupIssuesByStatus([
      issue({ key: "first", status: "To Do" }),
      issue({ key: "second", status: "To Do" }),
    ]);
    expect(groups[0].issues.map((i) => i.key)).toEqual(["first", "second"]);
  });
  it("empty in, empty out", () => {
    expect(groupIssuesByStatus([])).toEqual([]);
  });
});

describe("priorityLevel (visual tier from customizable names)", () => {
  it("classifies common Jira priority names", () => {
    expect(priorityLevel("Highest")).toBe("critical");
    expect(priorityLevel("Blocker")).toBe("critical");
    expect(priorityLevel("High")).toBe("high");
    expect(priorityLevel("Medium")).toBe("medium");
    expect(priorityLevel("Low")).toBe("low");
    expect(priorityLevel("Lowest")).toBe("low");
    expect(priorityLevel(null)).toBe("none");
    expect(priorityLevel("Weird Custom")).toBe("none");
  });
});

describe("dueState (day-granular semaphore)", () => {
  const now = new Date(2026, 6, 22, 15, 0).getTime(); // Jul 22 2026, 3pm local
  it("classifies against local days, not raw hours", () => {
    expect(dueState("2026-07-21", now)).toBe("overdue");
    expect(dueState("2026-07-22", now)).toBe("today");
    expect(dueState("2026-07-24", now)).toBe("soon");
    expect(dueState("2026-08-15", now)).toBe("later");
    expect(dueState(null, now)).toBeNull();
    expect(dueState("garbage", now)).toBeNull();
  });
});

describe("typeDotClass", () => {
  it("maps common types and falls back", () => {
    expect(typeDotClass("Bug")).toBe("type-bug");
    expect(typeDotClass("Story")).toBe("type-story");
    expect(typeDotClass("Task")).toBe("type-task");
    expect(typeDotClass("Sub-task")).toBe("type-task");
    expect(typeDotClass("Epic")).toBe("type-epic");
    expect(typeDotClass("Spike")).toBe("type-other");
  });
});

describe("formatSecondsForWorklog (rounds UP to 5m)", () => {
  it("formats and rounds up", () => {
    expect(formatSecondsForWorklog(45 * 60)).toBe("45m");
    expect(formatSecondsForWorklog(2 * 3600 + 11 * 60)).toBe("2h 15m");
    expect(formatSecondsForWorklog(3600)).toBe("1h");
    expect(formatSecondsForWorklog(61)).toBe("5m");
    expect(formatSecondsForWorklog(7 * 3600 + 56 * 60)).toBe("8h");
  });
});
