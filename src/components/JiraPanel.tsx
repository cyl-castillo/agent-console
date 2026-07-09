import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useJiraStore } from "../stores/jiraStore";
import { startSessionForIssue } from "../lib/startSessionForIssue";
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
    >↻</button>
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
        Your API token is stored in your OS keychain, never in a file or log — it
        only leaves this machine as an authenticated request to your own Jira site.
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
          onKeyDown={(e) => { if (e.key === "Enter") submit(); }}
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
        >Get a token ↗</button>
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

  return (
    <div className="jira-list">
      <div className="jira-account">
        <span className="jira-account-who" title={status?.siteUrl}>{status?.email}</span>
        <button
          className="wb-link"
          onClick={() => { if (confirm("Disconnect Jira? The stored token is removed.")) void disconnect(); }}
        >disconnect</button>
      </div>

      {issuesError && <PanelError message={issuesError} onRetry={() => void refreshIssues()} />}

      {loadingIssues && issues.length === 0 ? (
        <div className="wb-hint">Loading assigned issues…</div>
      ) : issues.length === 0 && !issuesError ? (
        <div className="wb-empty">No open issues assigned to you. Nice.</div>
      ) : (
        <ul className="jira-issues">
          {issues.map((it) => <IssueRow key={it.key} issue={it} />)}
        </ul>
      )}
    </div>
  );
}

function IssueRow({ issue }: { issue: JiraIssue }) {
  return (
    <li className="jira-issue" title={issue.summary}>
      <div className="jira-issue-top">
        <button
          className="jira-key"
          onClick={() => void openUrl(issue.url)}
          title={`Open ${issue.key} in Jira`}
        >{issue.key}</button>
        <span className={`jira-status cat-${issue.statusCategory}`}>{issue.status}</span>
        {issue.dueDate && <span className="jira-due" title="Due date">⏱ {issue.dueDate}</span>}
      </div>
      <div className="jira-summary">{issue.summary}</div>
      <div className="jira-issue-bottom">
        <div className="jira-meta">
          <span>{issue.issueType}</span>
          {issue.priority && <span> · {issue.priority}</span>}
          <span> · {issue.project}</span>
        </div>
        <button
          className="jira-start"
          onClick={() => startSessionForIssue(issue)}
          title={`Start an agent session for ${issue.key}, seeded with the ticket`}
        >▸ Start session</button>
      </div>
    </li>
  );
}
