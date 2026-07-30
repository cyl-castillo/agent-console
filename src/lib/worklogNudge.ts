import { ipc } from "../ipc/tauri";
import { localDay } from "../stores/jiraStore";
import { useToastStore } from "../stores/toastStore";
import { formatSecondsForWorklog } from "./jira";

const NUDGE_KEY = "agent-console:worklog-nudge";

/// Once per calendar day, on the first launch with a project open: if
/// yesterday saw witnessed ticket work that was never logged, say so. This is
/// the whole point of the digest — no more end-of-month archaeology.
export async function maybeWorklogNudge(projectRoot: string): Promise<void> {
  const today = localDay(0).date;
  try {
    if (localStorage.getItem(NUDGE_KEY) === today) return;
  } catch {
    /* ignore */
  }
  try {
    const status = await ipc.jiraStatus();
    if (!status.configured) return;
    const y = localDay(1);
    const digest = await ipc.jiraDailyDigest(projectRoot, y.startMs, y.endMs, y.date);
    const pending = digest.filter((e) => !e.logged);
    if (pending.length > 0) {
      const total = pending.reduce((acc, e) => acc + e.seconds, 0);
      useToastStore
        .getState()
        .show(
          `Yesterday left ${formatSecondsForWorklog(total)} unlogged across ${pending.length} ticket${pending.length === 1 ? "" : "s"} — open Tasks → Witnessed time`,
          "info",
        );
    }
    try {
      localStorage.setItem(NUDGE_KEY, today);
    } catch {
      /* ignore */
    }
  } catch {
    /* the nudge is a courtesy — never surface its failures */
  }
}
