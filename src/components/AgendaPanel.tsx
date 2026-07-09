import { useEffect, useMemo } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useJiraStore } from "../stores/jiraStore";
import { useSchedulerStore } from "../stores/schedulerStore";
import { startSessionForIssue } from "../lib/startSessionForIssue";
import {
  buildAgenda,
  BUCKET_LABELS,
  BUCKET_ORDER,
  type AgendaBucket,
  type AgendaItem,
} from "../lib/agenda";

/// Switch the right-hand workbench to another tab (the strip listens for this).
function openWorkbenchTab(tab: string) {
  window.dispatchEvent(new CustomEvent("ac:open-workbench-tab", { detail: tab }));
}

export function AgendaPanel() {
  const issues = useJiraStore((s) => s.issues);
  const jiraConfigured = useJiraStore((s) => s.status?.configured ?? false);
  const loadStatus = useJiraStore((s) => s.loadStatus);
  const jobs = useSchedulerStore((s) => s.jobs);

  // Opening Agenda without visiting Tasks first still needs Jira loaded.
  useEffect(() => {
    if (useJiraStore.getState().status === null) void loadStatus();
  }, [loadStatus]);

  const items = useMemo(() => buildAgenda(issues, jobs, Date.now()), [issues, jobs]);

  const grouped = useMemo(() => {
    const map = new Map<AgendaBucket, AgendaItem[]>();
    for (const it of items) {
      const arr = map.get(it.bucket) ?? [];
      arr.push(it);
      map.set(it.bucket, arr);
    }
    return map;
  }, [items]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">agenda</span>
      </div>
      <div className="workbench-body">
        {items.length === 0 ? (
          <div className="wb-empty">
            Nothing scheduled. Due dates from your Jira tasks and upcoming
            scheduled jobs show up here.
            {!jiraConfigured && (
              <>
                {" "}
                <button className="wb-link" onClick={() => openWorkbenchTab("jira")}>
                  Connect Jira in Tasks
                </button>{" "}
                to see ticket due dates.
              </>
            )}
          </div>
        ) : (
          BUCKET_ORDER.filter((b) => grouped.has(b)).map((b) => (
            <section key={b} className="agenda-group">
              <div className={`agenda-group-title ${b === "overdue" ? "overdue" : ""}`}>
                {BUCKET_LABELS[b]}
              </div>
              <ul className="agenda-items">
                {grouped.get(b)!.map((it) => <AgendaRow key={it.id} item={it} />)}
              </ul>
            </section>
          ))
        )}
      </div>
    </div>
  );
}

function AgendaRow({ item }: { item: AgendaItem }) {
  const when = new Date(item.whenMs);
  const label = item.allDay
    ? when.toLocaleDateString(undefined, { month: "short", day: "numeric" })
    : when.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

  return (
    <li className={`agenda-item kind-${item.kind}`}>
      <span className="agenda-when">{label}</span>
      <div className="agenda-main">
        <div className="agenda-title">{item.title}</div>
        <div className="agenda-sub">{item.subtitle}</div>
      </div>
      {item.kind === "issue" && item.issue && (
        <div className="agenda-actions">
          <button
            className="agenda-act"
            onClick={() => startSessionForIssue(item.issue!)}
            title="Start an agent session for this ticket"
          >▸ session</button>
          <button
            className="agenda-act"
            onClick={() => void openUrl(item.issue!.url)}
            title="Open in Jira"
          >↗</button>
        </div>
      )}
      {item.kind === "job" && (
        <button
          className="agenda-act"
          onClick={() => openWorkbenchTab("schedule")}
          title="Open the Schedule panel"
        >⧉</button>
      )}
    </li>
  );
}
