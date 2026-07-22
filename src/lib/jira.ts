import type { JiraIssue } from "../types/domain";

/// Who is driving the console for this project. Changes which tickets the
/// Queue shows (PO/PM see the whole project; everyone else their own) and
/// what the seeded session sets out to do.
export type ProjectRole = "developer" | "qa" | "analyst" | "po" | "pm";

export const ROLE_LABELS: Record<ProjectRole, string> = {
  developer: "Developer",
  qa: "QA",
  analyst: "Functional Analyst",
  po: "PO",
  pm: "PM",
};

const JQL_OWN =
  "assignee = currentUser() AND statusCategory != Done ORDER BY duedate ASC, priority DESC, updated DESC";
const JQL_ALL = "statusCategory != Done ORDER BY priority DESC, duedate ASC, updated DESC";

/// The JQL preset a role starts from. Always shown editable in the panel —
/// Jira workflows are project-customizable, so presets are starting points,
/// not gospel.
export function jqlForRole(role: ProjectRole): string {
  return role === "po" || role === "pm" ? JQL_ALL : JQL_OWN;
}

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
export function seedForIssue(issue: JiraIssue, role: ProjectRole = "developer"): string {
  const ctx = context(issue);
  // Role trumps stage: a QA opening any ticket is there to verify it; an
  // analyst to pin the requirement down; PO/PM to shape or report it.
  switch (role) {
    case "qa":
      return (
        `I need to verify Jira issue ${ctx}\n\n` +
        `Help me test this properly: derive a test plan from the acceptance criteria, run and ` +
        `extend the relevant tests, and probe edge cases. Report what passes, what fails, and ` +
        `what isn't covered.`
      );
    case "analyst":
      return (
        `I'm analyzing Jira issue ${ctx}\n\n` +
        `Help me pin the requirement down: what the change actually asks for, which parts of the ` +
        `system it touches, open questions to raise, and a draft of functional acceptance ` +
        `criteria. Read the relevant code to ground it in reality — don't change anything.`
      );
    case "po":
      return (
        `I'm refining Jira issue ${ctx}\n\n` +
        `Help me get this story ready: sharpen the description, write or tighten the acceptance ` +
        `criteria, flag hidden complexity by reading the relevant code, and propose a split if ` +
        `it's too big. Don't change any code.`
      );
    case "pm":
      return (
        `I need a status picture of Jira issue ${ctx}\n\n` +
        `Read the repo (branches, recent commits, open changes) and tell me the REAL state of ` +
        `this work: what's done, what's in flight, what's blocked or risky. Summarize it in ` +
        `plain language I can share with stakeholders. Don't change anything.`
      );
    default:
      break;
  }
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
  return {
    implement: "implementation",
    continue: "continue",
    review: "review",
    test: "testing",
    debug: "debugging",
  }[intent];
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

/// Normalized priority tier for visual treatment. Jira priority names are
/// project-customizable, so classify by keywords with a neutral fallback.
export type PriorityLevel = "critical" | "high" | "medium" | "low" | "none";

export function priorityLevel(priority: string | null | undefined): PriorityLevel {
  const p = (priority ?? "").toLowerCase();
  if (!p) return "none";
  if (/highest|blocker|critical|urgent|p0/.test(p)) return "critical";
  if (/high|major|p1/.test(p)) return "high";
  if (/lowest|low|minor|trivial|p[34]/.test(p)) return "low";
  if (/medium|normal|p2/.test(p)) return "medium";
  return "none";
}

/// Due-date urgency for the semaphore. Day-granular, local time.
export type DueState = "overdue" | "today" | "soon" | "later";

export function dueState(due: string | null | undefined, nowMs: number): DueState | null {
  if (!due) return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(due);
  if (!m) return null;
  const dueDay = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getTime();
  const now = new Date(nowMs);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const DAY = 86_400_000;
  if (dueDay < today) return "overdue";
  if (dueDay === today) return "today";
  if (dueDay <= today + 3 * DAY) return "soon";
  return "later";
}

/// CSS modifier for the colored type dot. Same keyword approach as intents.
export function typeDotClass(issueType: string): string {
  const t = issueType.toLowerCase();
  if (/bug|defect|incident|hotfix/.test(t)) return "type-bug";
  if (/story/.test(t)) return "type-story";
  if (/epic/.test(t)) return "type-epic";
  if (/task|sub/.test(t)) return "type-task";
  return "type-other";
}

/// Seconds → worklog-friendly label, rounded UP to 5-minute granularity
/// ("2h 15m", "45m"). Suggestion display + one-click fill share this.
export function formatSecondsForWorklog(seconds: number): string {
  const s5 = Math.ceil(seconds / 300) * 300;
  const h = Math.floor(s5 / 3600);
  const m = Math.round((s5 % 3600) / 60);
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}
