import { describe, expect, it } from "vitest";

import { buildAgenda, bucketFor, parseDueLocal } from "./agenda";
import type { JiraIssue, Job } from "../types/domain";

// A fixed "now": 2026-07-09 12:00 local.
const NOW = new Date(2026, 6, 9, 12, 0, 0).getTime();
const DAY = 86_400_000;

function issue(over: Partial<JiraIssue> = {}): JiraIssue {
  return {
    key: "ABC-1", summary: "s", status: "To Do", statusCategory: "new",
    priority: "High", issueType: "Bug", dueDate: null, project: "Acme",
    updated: null, url: "https://x/browse/ABC-1", ...over,
  };
}
function job(over: Partial<Job> = {}): Job {
  return {
    id: "j1", name: "nightly", enabled: true,
    trigger: {} as Job["trigger"], action: {} as Job["action"],
    onMissed: "skip" as Job["onMissed"], cooldownMs: 0, createdAtMs: 0,
    consecutiveFailures: 0, ...over,
  };
}

describe("parseDueLocal", () => {
  it("reads YYYY-MM-DD as local midnight (no UTC day shift)", () => {
    const ms = parseDueLocal("2026-07-15")!;
    const d = new Date(ms);
    expect(d.getFullYear()).toBe(2026);
    expect(d.getMonth()).toBe(6);
    expect(d.getDate()).toBe(15);
    expect(d.getHours()).toBe(0);
  });
  it("returns null on garbage", () => {
    expect(parseDueLocal("soon")).toBeNull();
  });
});

describe("bucketFor", () => {
  it("classifies relative days", () => {
    expect(bucketFor(NOW - DAY, NOW)).toBe("overdue");
    expect(bucketFor(NOW, NOW)).toBe("today");
    expect(bucketFor(NOW + DAY, NOW)).toBe("tomorrow");
    expect(bucketFor(NOW + 4 * DAY, NOW)).toBe("week");
    expect(bucketFor(NOW + 30 * DAY, NOW)).toBe("later");
  });
  it("earlier today still counts as today, not overdue", () => {
    expect(bucketFor(NOW - 3 * 3_600_000, NOW)).toBe("today");
  });
});

describe("buildAgenda", () => {
  it("merges issues with due dates and enabled timed jobs, sorted by time", () => {
    const items = buildAgenda(
      [issue({ key: "A", dueDate: "2026-07-10" }), issue({ key: "B", dueDate: null })],
      [job({ id: "j", nextDueMs: NOW + 3600_000 })],
      NOW,
    );
    // B has no due date → excluded. A (tomorrow) + job (today, sooner) → 2 items.
    expect(items.map((i) => i.id)).toEqual(["job:j", "issue:A"]);
  });

  it("skips disabled jobs and jobs without a next run", () => {
    const items = buildAgenda(
      [],
      [job({ id: "off", enabled: false, nextDueMs: NOW + 1000 }), job({ id: "norun", nextDueMs: undefined })],
      NOW,
    );
    expect(items).toHaveLength(0);
  });

  it("tags issue items with the issue so the row can act on it", () => {
    const [item] = buildAgenda([issue({ key: "X", dueDate: "2026-07-09" })], [], NOW);
    expect(item.kind).toBe("issue");
    expect(item.issue?.key).toBe("X");
    expect(item.bucket).toBe("today");
  });
});
