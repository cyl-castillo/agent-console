import type { JiraIssue } from "../types/domain";

/// What the user is most likely doing with a ticket, inferred from its stage.
/// Drives the prompt an agent session is seeded with — reviewing a ticket in
/// "Code Review" is a different job than implementing one in "To Do".
export type IssueIntent = "implement" | "continue" | "review" | "test" | "debug";

function isBugType(issueType: string): boolean {
  return /bug|defect|incident|hotfix/i.test(issueType);
}

/// Infer the intent. Status *names* are project-customizable, so we match on
/// keywords in the name and fall back to the standard statusCategory
/// ("new" | "indeterminate" | "done"). Review/test win over type; otherwise a
/// bug leans to debugging.
export function intentForIssue(issue: JiraIssue): IssueIntent {
  const name = issue.status.toLowerCase();
  if (/review|approval|pr\b/.test(name)) return "review";
  if (/test|qa|verif|accept/.test(name)) return "test";
  if (isBugType(issue.issueType)) return "debug";
  if (issue.statusCategory === "new") return "implement";
  return "continue"; // indeterminate, non-bug: it's mid-flight
}

function context(issue: JiraIssue): string {
  const bits = [issue.issueType, issue.priority ? `${issue.priority} priority` : null]
    .filter(Boolean)
    .join(", ");
  return `${issue.key}: ${issue.summary}` + (bits ? ` (${bits}).` : ".") + ` Ref: ${issue.url}`;
}

/// The prompt the agent's input is seeded with. Stage-aware, so a Code Review
/// ticket asks for a review, not an implementation. Typed without a trailing
/// newline — the user reviews and extends it before sending.
export function seedForIssue(issue: JiraIssue): string {
  const ctx = context(issue);
  switch (intentForIssue(issue)) {
    case "review":
      return (
        `I need to review Jira issue ${ctx}\n\n` +
        `Find the changes for this ticket (its branch, open PR, or recent commits) and ` +
        `review them: correctness, edge cases, error handling, test coverage, and code quality. ` +
        `Summarize what the change does and flag anything that should block approval.`
      );
    case "test":
      return (
        `I need to verify Jira issue ${ctx}\n\n` +
        `Help me test this: work out what to check against the acceptance criteria, run and ` +
        `extend the relevant tests, and probe the edge cases. Report what passes and what doesn't.`
      );
    case "debug":
      return (
        `I'm working on bug ${ctx}\n\n` +
        `Help me reproduce and diagnose it first — find the relevant code and understand the ` +
        `failure — then propose a fix. Don't change anything until we agree on the cause.`
      );
    case "continue":
      return (
        `I'm continuing Jira issue ${ctx}\n\n` +
        `This is already in progress. Check what's been done so far (recent commits, the current ` +
        `diff) and help me pick up from there.`
      );
    case "implement":
    default:
      return (
        `I'm working on Jira issue ${ctx}\n\n` +
        `Help me plan and implement this. Start by exploring the relevant code.`
      );
  }
}

/// A one-word verb for the intent, for button tooltips ("Start a review session").
export function intentVerb(intent: IssueIntent): string {
  return { implement: "implementation", continue: "continue", review: "review", test: "testing", debug: "debugging" }[intent];
}

export interface IssueGroup {
  status: string;
  statusCategory: string;
  issues: JiraIssue[];
}

// Workflow order for the standard status categories; unknown → middle.
const CATEGORY_RANK: Record<string, number> = { new: 0, indeterminate: 1, done: 2 };

/// Group assigned issues by their status (e.g. "To Do", "In Progress", "In
/// Review"), ordered by workflow stage then name — so "what to build" and "what
/// to review" read as separate sections. Within a group, incoming order (due
/// date, from the JQL) is preserved.
export function groupIssuesByStatus(issues: JiraIssue[]): IssueGroup[] {
  const map = new Map<string, JiraIssue[]>();
  for (const it of issues) {
    const arr = map.get(it.status);
    if (arr) arr.push(it);
    else map.set(it.status, [it]);
  }
  return [...map.entries()]
    .map(([status, list]) => ({ status, statusCategory: list[0].statusCategory, issues: list }))
    .sort(
      (a, b) =>
        (CATEGORY_RANK[a.statusCategory] ?? 1) - (CATEGORY_RANK[b.statusCategory] ?? 1) ||
        a.status.localeCompare(b.status),
    );
}
