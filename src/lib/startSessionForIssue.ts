import { DEFAULT_AGENT } from "../agents/profiles";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useModelStore } from "../stores/modelStore";
import { useUIStore } from "../stores/uiStore";
import { useToastStore } from "../stores/toastStore";
import type { JiraIssue } from "../types/domain";

/// The prompt the agent's input is seeded with. Kept short and factual — the
/// user reviews and extends it before sending (it's typed without a newline).
export function seedForIssue(issue: JiraIssue): string {
  const bits = [issue.issueType, issue.priority ? `${issue.priority} priority` : null]
    .filter(Boolean)
    .join(", ");
  return (
    `I'm working on Jira issue ${issue.key}: ${issue.summary}` +
    (bits ? ` (${bits}).` : ".") +
    ` Ref: ${issue.url}\n\nHelp me plan and implement this. Start by exploring the relevant code.`
  );
}

/// The bridge: turn an assigned ticket into a new agent session in the current
/// project, named after the ticket and seeded with its context. Reuses the last
/// agent/model chosen for this project so it's one click, not a chooser.
export function startSessionForIssue(issue: JiraIssue): void {
  const project = useSessionStore.getState().project;
  if (!project) {
    useToastStore.getState().show("Open a project first to start a session", "error");
    return;
  }
  const models = useModelStore.getState();
  const agent = models.defaultAgentFor(project.root) ?? DEFAULT_AGENT;
  const model = models.defaultFor(project.root, agent);

  const terminals = useTerminalsStore.getState();
  terminals.add(project.root, issue.key, model, agent, undefined, undefined, seedForIssue(issue));
  terminals.persist();

  useUIStore.getState().setTab("terminal");
  useToastStore
    .getState()
    .show(`Session ${issue.key} started — review the prompt, then send`, "success");
}
