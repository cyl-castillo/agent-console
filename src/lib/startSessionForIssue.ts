import { DEFAULT_AGENT } from "../agents/profiles";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useModelStore } from "../stores/modelStore";
import { useUIStore } from "../stores/uiStore";
import { useToastStore } from "../stores/toastStore";
import { ipc } from "../ipc/tauri";
import { seedForIssue, intentForIssue, intentVerb } from "./jira";
import type { JiraIssue } from "../types/domain";

export interface StartOptions {
  /// Run the session in its own isolated git worktree, instead of the project
  /// checkout. The branch name is `branch` (the user-confirmed, convention-aware
  /// name); when omitted the backend falls back to `agent/<key>`.
  worktree?: boolean;
  branch?: string;
}

/// The bridge: turn an assigned ticket into a new agent session, named after the
/// ticket and seeded with a stage-aware prompt. Reuses the last agent/model
/// chosen for this project so it's one click, not a chooser. With
/// `worktree: true` the agent works on an isolated branch derived from the
/// ticket key, so the ticket → branch → agent mapping is 1:1.
export async function startSessionForIssue(
  issue: JiraIssue,
  opts: StartOptions = {},
): Promise<void> {
  const project = useSessionStore.getState().project;
  if (!project) {
    useToastStore.getState().show("Open a project first to start a session", "error");
    return;
  }
  const models = useModelStore.getState();
  const agent = models.defaultAgentFor(project.root) ?? DEFAULT_AGENT;
  const model = models.defaultFor(project.root, agent);
  const seed = seedForIssue(issue);
  const verb = intentVerb(intentForIssue(issue));
  const terminals = useTerminalsStore.getState();
  const toast = useToastStore.getState();

  if (opts.worktree) {
    try {
      // Explicit, user-confirmed branch (convention-aware); base defaults to
      // the current branch on the backend.
      const created = await ipc.worktreeCreate(issue.key, undefined, opts.branch);
      terminals.add(
        created.info.path,
        issue.key,
        model,
        agent,
        created.info,
        created.setupCommand ?? undefined,
        seed,
      );
      const copies = created.copied.length ? ` · copied ${created.copied.join(", ")}` : "";
      toast.show(
        `${issue.key} (${verb}) on ${created.info.branch}${copies} — review the prompt, then send`,
        "success",
      );
    } catch (e) {
      toast.show(`Couldn't create a worktree for ${issue.key}: ${e}`, "error");
      return;
    }
  } else {
    terminals.add(project.root, issue.key, model, agent, undefined, undefined, seed);
    toast.show(`Session ${issue.key} started (${verb}) — review the prompt, then send`, "success");
  }

  terminals.persist();
  useUIStore.getState().setTab("terminal");
}
