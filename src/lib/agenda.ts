import type { JiraIssue, Job } from "../types/domain";

export type AgendaKind = "issue" | "job";
export type AgendaBucket = "overdue" | "today" | "tomorrow" | "week" | "later";

export interface AgendaItem {
  id: string;
  kind: AgendaKind;
  whenMs: number;
  /// Day-granular items (Jira due dates) render a date; timed items (jobs) a time.
  allDay: boolean;
  title: string;
  subtitle: string;
  bucket: AgendaBucket;
  /// Present for issue items so the row can act on the ticket.
  issue?: JiraIssue;
  jobId?: string;
}

/// Parse a Jira "YYYY-MM-DD" due date as LOCAL midnight. `new Date(str)` would
/// read it as UTC and shift a day in western timezones, so build it by parts.
export function parseDueLocal(due: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(due);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getTime();
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/// Which day-bucket a timestamp falls into relative to `nowMs`.
export function bucketFor(whenMs: number, nowMs: number): AgendaBucket {
  const today = startOfDay(nowMs);
  const day = startOfDay(whenMs);
  const DAY = 86_400_000;
  if (day < today) return "overdue";
  if (day === today) return "today";
  if (day === today + DAY) return "tomorrow";
  if (day <= today + 7 * DAY) return "week";
  return "later";
}

/// Merge assigned Jira issues (by due date) and enabled scheduler jobs (by next
/// run) into one chronological agenda. Items without a time are skipped — an
/// agenda is only about things that land on the clock.
export function buildAgenda(issues: JiraIssue[], jobs: Job[], nowMs: number): AgendaItem[] {
  const items: AgendaItem[] = [];

  for (const it of issues) {
    if (!it.dueDate) continue;
    const whenMs = parseDueLocal(it.dueDate);
    if (whenMs == null) continue;
    items.push({
      id: `issue:${it.key}`,
      kind: "issue",
      whenMs,
      allDay: true,
      title: `${it.key} · ${it.summary}`,
      subtitle: [it.issueType, it.priority, it.project].filter(Boolean).join(" · "),
      bucket: bucketFor(whenMs, nowMs),
      issue: it,
    });
  }

  for (const j of jobs) {
    if (!j.enabled || j.nextDueMs == null) continue;
    items.push({
      id: `job:${j.id}`,
      kind: "job",
      whenMs: j.nextDueMs,
      allDay: false,
      title: j.name,
      subtitle: "Scheduled job",
      bucket: bucketFor(j.nextDueMs, nowMs),
      jobId: j.id,
    });
  }

  return items.sort((a, b) => a.whenMs - b.whenMs);
}

export const BUCKET_LABELS: Record<AgendaBucket, string> = {
  overdue: "Overdue",
  today: "Today",
  tomorrow: "Tomorrow",
  week: "This week",
  later: "Later",
};

export const BUCKET_ORDER: AgendaBucket[] = ["overdue", "today", "tomorrow", "week", "later"];
