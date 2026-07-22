import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { ipc } from "../ipc/tauri";
import { useJiraStore } from "../stores/jiraStore";
import { useToastStore } from "../stores/toastStore";
import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { useRoleStore } from "../stores/roleStore";
import { startSessionForIssue } from "../lib/startSessionForIssue";
import {
  dueState,
  formatSecondsForWorklog,
  groupIssuesByStatus,
  intentForIssue,
  intentVerb,
  jqlForRole,
  priorityLevel,
  ROLE_LABELS,
  typeDotClass,
  type ProjectRole,
} from "../lib/jira";
import { PanelError } from "./PanelError";
import type { JiraIssue } from "../types/domain";

export function JiraPanel() {
  const status = useJiraStore((s) => s.status);
  const loadingStatus = useJiraStore((s) => s.loadingStatus);
  const loadStatus = useJiraStore((s) => s.loadStatus);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">tasks</span>
        {status?.configured && <JiraHeaderActions />}
      </div>
      <div className="workbench-body">
        {loadingStatus && !status ? (
          <div className="wb-hint">Loading…</div>
        ) : status?.configured ? (
          <IssueList />
        ) : (
          <ConnectForm />
        )}
      </div>
    </div>
  );
}

function JiraHeaderActions() {
  const refreshIssues = useJiraStore((s) => s.refreshIssues);
  const loadingIssues = useJiraStore((s) => s.loadingIssues);
  return (
    <button
      className="workbench-action"
      onClick={() => void refreshIssues()}
      disabled={loadingIssues}
      title="Refresh assigned issues"
    >
      ↻
    </button>
  );
}

function ConnectForm() {
  const connect = useJiraStore((s) => s.connect);
  const connecting = useJiraStore((s) => s.connecting);
  const connectError = useJiraStore((s) => s.connectError);
  const [siteUrl, setSiteUrl] = useState("https://");
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");

  const submit = () => {
    if (connecting) return;
    void connect(siteUrl.trim(), email.trim(), token.trim());
  };

  return (
    <div className="jira-connect">
      <p className="wb-hint wb-trust">
        Your API token is stored in your OS keychain, never in a file or log — it only leaves this
        machine as an authenticated request to your own Jira site.
      </p>

      <label className="jira-field">
        <span>Site URL</span>
        <input
          className="jira-input"
          value={siteUrl}
          onChange={(e) => setSiteUrl(e.target.value)}
          placeholder="https://yourteam.atlassian.net"
          spellCheck={false}
          autoCapitalize="off"
        />
      </label>
      <label className="jira-field">
        <span>Email</span>
        <input
          className="jira-input"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@company.com"
          spellCheck={false}
          autoCapitalize="off"
        />
      </label>
      <label className="jira-field">
        <span>API token</span>
        <input
          className="jira-input"
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
          placeholder="paste your Atlassian API token"
          spellCheck={false}
        />
      </label>

      <div className="jira-connect-actions">
        <button
          className="jira-token-link"
          onClick={() =>
            void openUrl("https://id.atlassian.com/manage-profile/security/api-tokens")
          }
          title="Create an API token on id.atlassian.com"
        >
          Get a token ↗
        </button>
        <button
          className="wb-cta wb-cta-sm"
          onClick={submit}
          disabled={connecting || !token.trim() || !email.trim()}
        >
          {connecting ? "Connecting…" : "Connect"}
        </button>
      </div>

      {connectError && <PanelError message={connectError} />}
    </div>
  );
}

function IssueList() {
  const issues = useJiraStore((s) => s.issues);
  const loadingIssues = useJiraStore((s) => s.loadingIssues);
  const issuesError = useJiraStore((s) => s.issuesError);
  const refreshIssues = useJiraStore((s) => s.refreshIssues);
  const disconnect = useJiraStore((s) => s.disconnect);
  const status = useJiraStore((s) => s.status);
  const [filter, setFilter] = useState("");
  // The worktree action only makes sense in a git repo.
  const isRepo = useChangesStore((s) => s.status?.isRepo ?? false);
  const projectRoot = useSessionStore((s) => s.project?.root ?? null);
  const roleFor = useRoleStore((s) => s.roleFor);
  const setRoleFor = useRoleStore((s) => s.setRoleFor);
  const jqlFor = useRoleStore((s) => s.jqlFor);
  const hasCustomJql = useRoleStore((s) => s.hasCustomJql);
  const setJqlFor = useRoleStore((s) => s.setJqlFor);
  const setJql = useJiraStore((s) => s.setJql);
  const role = projectRoot ? roleFor(projectRoot) : "developer";
  const effectiveJql = projectRoot ? jqlFor(projectRoot, role) : jqlForRole(role);
  const [jqlOpen, setJqlOpen] = useState(false);
  const [jqlDraft, setJqlDraft] = useState<string | null>(null);

  // The store fetches with whatever JQL the current (project, role) demands;
  // this also covers the very first load and role/JQL switches.
  useEffect(() => {
    void setJql(effectiveJql);
  }, [effectiveJql, setJql]);

  const pickRole = (r: ProjectRole) => {
    if (!projectRoot) return;
    setRoleFor(projectRoot, r);
    setJqlDraft(null);
  };

  const q = filter.trim().toLowerCase();
  const visible = q
    ? issues.filter(
        (i) =>
          i.key.toLowerCase().includes(q) ||
          i.summary.toLowerCase().includes(q) ||
          i.project.toLowerCase().includes(q),
      )
    : issues;

  return (
    <div className="jira-list">
      <div className="jira-account">
        <span className="jira-account-who" title={status?.siteUrl}>
          {status?.email}
        </span>
        <button
          className="wb-link"
          onClick={() => {
            if (confirm("Disconnect Jira? The stored token is removed.")) void disconnect();
          }}
        >
          disconnect
        </button>
      </div>

      <div className="jira-role-row">
        <span className="jira-role-label">role</span>
        {(Object.keys(ROLE_LABELS) as ProjectRole[]).map((r) => (
          <button
            key={r}
            className={`jira-role-chip ${role === r ? "active" : ""}`}
            onClick={() => pickRole(r)}
            title={
              r === "po" || r === "pm"
                ? `${ROLE_LABELS[r]} — sees the whole project`
                : `${ROLE_LABELS[r]} — sees their own assigned issues`
            }
          >
            {ROLE_LABELS[r]}
          </button>
        ))}
        <button
          className={`jira-role-adv ${jqlOpen ? "open" : ""} ${projectRoot && hasCustomJql(projectRoot, role) ? "custom" : ""}`}
          onClick={() => {
            setJqlOpen((v) => !v);
            setJqlDraft(null);
          }}
          title="Show / edit the JQL this view uses"
        >
          jql
        </button>
      </div>
      {jqlOpen && (
        <div className="jira-jql-editor">
          <textarea
            className="jira-jql-input"
            rows={2}
            spellCheck={false}
            value={jqlDraft ?? effectiveJql}
            onChange={(e) => setJqlDraft(e.target.value)}
          />
          <div className="jira-jql-actions">
            <button
              className="wb-link"
              onClick={() => {
                if (!projectRoot) return;
                setJqlFor(projectRoot, role, jqlDraft);
                setJqlDraft(null);
              }}
              disabled={jqlDraft === null || !jqlDraft.trim()}
            >
              save
            </button>
            <button
              className="wb-link"
              onClick={() => {
                if (!projectRoot) return;
                setJqlFor(projectRoot, role, null);
                setJqlDraft(null);
              }}
              title="Back to this role's preset"
            >
              reset to preset
            </button>
          </div>
        </div>
      )}

      {issuesError && <PanelError message={issuesError} onRetry={() => void refreshIssues()} />}

      {issues.length > 3 && (
        <input
          className="jira-filter"
          value={filter}
          placeholder="Filter by key, summary, project…"
          spellCheck={false}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setFilter("");
          }}
        />
      )}

      {loadingIssues && issues.length === 0 ? (
        <div className="wb-hint">Loading assigned issues…</div>
      ) : issues.length === 0 && !issuesError ? (
        <div className="wb-empty">No open issues assigned to you. Nice.</div>
      ) : visible.length === 0 ? (
        <div className="wb-empty">Nothing matches “{filter.trim()}”.</div>
      ) : (
        groupIssuesByStatus(visible).map((g) => (
          <section key={g.status} className="jira-group">
            <div className={`jira-group-title cat-${g.statusCategory}`}>
              {g.status}
              <span className="jira-group-count">{g.issues.length}</span>
            </div>
            <ul className="jira-issues">
              {g.issues.map((it) => (
                <IssueRow key={it.key} issue={it} isRepo={isRepo} />
              ))}
            </ul>
          </section>
        ))
      )}
    </div>
  );
}

/// Today as YYYY-MM-DD in local time (what a date input expects).
function todayISO(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function IssueRow({ issue, isRepo }: { issue: JiraIssue; isRepo: boolean }) {
  const verb = intentVerb(intentForIssue(issue));
  // null = editor closed. A string = open, holding the (editable) branch name.
  const [branch, setBranch] = useState<string | null>(null);
  const [proposing, setProposing] = useState(false);
  // Worklog editor state (null = closed).
  const [log, setLog] = useState<{ duration: string; date: string; comment: string } | null>(null);
  const [logging, setLogging] = useState(false);
  const logWork = useJiraStore((s) => s.logWork);
  const showToast = useToastStore((s) => s.show);
  const projectRoot = useSessionStore((s) => s.project?.root ?? null);
  // Assisted logging: what the Testigo ledger witnessed for this ticket on the
  // selected day. Suggestion only — it fills the field, never submits.
  const [suggest, setSuggest] = useState<{ seconds: number; events: number } | null>(null);
  const logDate = log?.date ?? null;
  useEffect(() => {
    setSuggest(null);
    if (!logDate || !projectRoot) return;
    const [y, m, d] = logDate.split("-").map(Number);
    if (!y || !m || !d) return;
    const start = new Date(y, m - 1, d).getTime();
    let alive = true;
    ipc
      .jiraWorklogSuggestion(projectRoot, issue.key, start, start + 86_400_000)
      .then((sug) => {
        if (alive) setSuggest(sug);
      })
      .catch(() => {
        if (alive) setSuggest(null);
      });
    return () => {
      alive = false;
    };
  }, [logDate, projectRoot, issue.key]);

  const submitLog = async () => {
    if (!log || logging) return;
    if (!log.duration.trim()) return;
    setLogging(true);
    const label = await logWork(issue.key, log.duration, log.date, log.comment || undefined);
    setLogging(false);
    if (label) {
      setLog(null);
      showToast(`Logged ${label} on ${issue.key}`, "success");
    } else {
      const err = useJiraStore.getState().logError ?? "unknown error";
      showToast(`Worklog failed: ${err.slice(0, 140)}`, "error");
    }
  };

  const openWorktreeEditor = async () => {
    setProposing(true);
    try {
      // Pre-fill from the project/skill convention (backend-resolved). Nothing
      // is created yet — the user confirms or edits first.
      const suggested = await ipc.worktreeSuggestBranch(issue.key, issue.summary, issue.issueType);
      setBranch(suggested || issue.key);
    } catch {
      setBranch(issue.key);
    } finally {
      setProposing(false);
    }
  };

  const createWorktree = () => {
    const b = (branch ?? "").trim();
    if (!b) return;
    setBranch(null);
    void startSessionForIssue(issue, { worktree: true, branch: b });
  };

  const prio = priorityLevel(issue.priority);
  const due = dueState(issue.dueDate, Date.now());

  return (
    <li className="jira-issue" title={issue.summary}>
      <div className="jira-issue-top">
        <span
          className={`jira-type-dot ${typeDotClass(issue.issueType)}`}
          title={issue.issueType}
          aria-hidden
        />
        <button
          className="jira-key"
          onClick={() => void openUrl(issue.url)}
          title={`Open ${issue.key} in Jira`}
        >
          {issue.key}
        </button>
        {prio !== "none" && prio !== "medium" && (
          <span className={`jira-prio prio-${prio}`} title={`${issue.priority} priority`}>
            {issue.priority}
          </span>
        )}
        {issue.dueDate && due && (
          <span className={`jira-due due-${due}`} title={`Due ${issue.dueDate}`}>
            {due === "overdue"
              ? `overdue · ${issue.dueDate}`
              : due === "today"
                ? "due today"
                : `⏱ ${issue.dueDate}`}
          </span>
        )}
      </div>
      <div className="jira-summary">{issue.summary}</div>

      {log !== null ? (
        <div className="jira-log-editor" onClick={(e) => e.stopPropagation()}>
          <input
            className="jira-log-duration"
            value={log.duration}
            placeholder="1h 30m"
            autoFocus
            spellCheck={false}
            onChange={(e) => setLog({ ...log, duration: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter") void submitLog();
              if (e.key === "Escape") setLog(null);
            }}
            title='Time spent — "1h 30m", "90m", "2h"'
          />
          <input
            className="jira-log-date"
            type="date"
            value={log.date}
            onChange={(e) => setLog({ ...log, date: e.target.value })}
            title="Day the work happened"
          />
          {suggest && (
            <button
              className="jira-log-suggest"
              onClick={() =>
                setLog((l) => l && { ...l, duration: formatSecondsForWorklog(suggest.seconds) })
              }
              title={`Witnessed activity for ${issue.key} that day: ${suggest.events} ledger events. Click to fill.`}
            >
              ◈ {formatSecondsForWorklog(suggest.seconds)}
            </button>
          )}
          <input
            className="jira-log-comment"
            value={log.comment}
            placeholder="comment (optional)"
            spellCheck={false}
            onChange={(e) => setLog({ ...log, comment: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter") void submitLog();
              if (e.key === "Escape") setLog(null);
            }}
          />
          <button
            className="jira-start"
            onClick={() => void submitLog()}
            disabled={logging || !log.duration.trim()}
            title={`Log this time on ${issue.key}`}
          >
            {logging ? "…" : "Log"}
          </button>
          <button className="jira-wt-cancel" onClick={() => setLog(null)} title="Cancel">
            ✕
          </button>
        </div>
      ) : branch !== null ? (
        <div className="jira-wt-editor" onClick={(e) => e.stopPropagation()}>
          <span className="jira-wt-label">branch</span>
          <input
            className="jira-wt-input"
            value={branch}
            autoFocus
            spellCheck={false}
            onChange={(e) => setBranch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") createWorktree();
              if (e.key === "Escape") setBranch(null);
            }}
          />
          <button
            className="jira-start"
            onClick={createWorktree}
            title="Create the worktree on this branch and start the session"
          >
            Create
          </button>
          <button className="jira-wt-cancel" onClick={() => setBranch(null)} title="Cancel">
            ✕
          </button>
        </div>
      ) : (
        <div className="jira-issue-bottom">
          <div className="jira-meta">
            <span>{issue.issueType}</span>
            <span> · {issue.project}</span>
          </div>
          <div className="jira-issue-actions">
            <button
              className="jira-start jira-start-log"
              onClick={() => setLog({ duration: "", date: todayISO(), comment: "" })}
              title={`Log time spent on ${issue.key}`}
            >
              ⏱ log
            </button>
            {isRepo && (
              <button
                className="jira-start jira-start-wt"
                onClick={() => void openWorktreeEditor()}
                disabled={proposing}
                title={`Start a ${verb} session in an isolated worktree for ${issue.key} (you name the branch)`}
              >
                {proposing ? "…" : "⎇ worktree"}
              </button>
            )}
            <button
              className="jira-start"
              onClick={() => void startSessionForIssue(issue)}
              title={`Start a ${verb} session for ${issue.key} in the project checkout`}
            >
              ▸ Start session
            </button>
          </div>
        </div>
      )}
    </li>
  );
}
