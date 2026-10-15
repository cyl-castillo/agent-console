# Action contract

Every action the console can perform has a stable id. The command palette,
spoken commands, the agent bridge (`window.__agentConsole`) and the console's
MCP server (D9) all address actions by these ids. Adding an id is a documented
change here; renaming or removing one is a breaking change for every client.
`src/actions/ids.ts` is the typed list and a test keeps it equal to the
palette registry and to this document.

## Navigation

| id | opens |
| --- | --- |
| `nav.terminal` | the Conversation tab of Sessions (the terminal is its drawer) |
| `nav.changes` | the Changes tab |
| `nav.preview` | the Preview tab |
| `nav.skills` | Coach → Skills |
| `nav.advisor` | Coach → Advisor |
| `nav.learning` | Coach → Learning |
| `nav.context` | Context (CLAUDE.md and memories) |
| `nav.permissions` | Trust → Permissions |
| `nav.vault` | Trust → Vault |
| `nav.plugins` | Add-ons → Plugins |
| `nav.mcp` | Add-ons → MCP servers |
| `nav.tasks` | Tasks → Jira queue |
| `nav.agenda` | Tasks → Agenda |
| `nav.teams` | Teams |
| `nav.notes` | Notes |
| `nav.proof` | Proof → Ledger |
| `nav.roundtable` | Room |
| `nav.jobs` | Jobs board |
| `nav.schedule` | Schedule |
| `nav.transfer` | Export / import |
| `nav.feedback` | Feedback |
| `modules.configure` | the Modules modal |

## Git

| id | does |
| --- | --- |
| `git.commit` | commit the staged changes |
| `git.push` | push the current branch |
| `git.open_pr` | open the PR URL for the current branch |
| `git.refresh` | refresh git status |

## Sessions and prompts

| id | does |
| --- | --- |
| `session.new` | start a new session in this project |
| `session.close` | close the active session (confirms when live) |
| `prompt.rerun_last` | type the last prompt again |
| `prompt.composer` | toggle the prompt composer |
| `snapshot.restore_latest` | restore the latest per-turn snapshot |
| `snapshot.undo_restore` | undo the last restore |

## Shell and app

| id | does |
| --- | --- |
| `ui.toggle_sidebar` | show/hide the list pane |
| `ui.toggle_right_panel` | show/hide the right panel |
| `ui.toggle_theme` | dark ↔ light |
| `project.copy_path` | copy the project path |
| `auth.claude_login` | open a `claude login` session |
| `auth.codex_login` | open a `codex login` session |
| `app.check_updates` | check for a new release |
| `app.report_problem` | open a prefilled GitHub issue |
| `setup.wizard` | the first-run wizard |
| `help.getting_started` | the Getting Started guide |

## The bridge

`window.__agentConsole` exposes `listActions()`, `invoke(id)` and `state()`
(project, sessions, pending decisions, destination). Reads come from
the stores; every write goes through an action. In the app it is reachable
from the console's own code and tests; the MCP server relays the same calls
from the loopback listener.

## The console as an MCP server

`hook-bridge console-mcp --project <root> --caller <name>` is an MCP stdio
server any agent can be pointed at (Add-ons → MCP servers shows the exact
`claude mcp add` command and the Codex snippet). It relays every call to the
console over the loopback listener, with the same token as the hooks. The
`--caller` name is the agent's identity: every call is witnessed in the
ledger as `agent:<name>` (kind `agent_action`), on the session's lane when
the call names one — never as the human.

| tool | does |
| --- | --- |
| `console_state` | project, sessions, pending decisions, destination |
| `list_actions` | the action contract above, with availability |
| `invoke_action` | run an allowed action (`nav.*`, `ui.*`, a few harmless app actions, `session.new`); kill/delete/push/spend are refused |
| `list_sessions` | sessions with attention state, agent, model, worktree, age |
| `new_session` | start a session (name, agent, model optional); returns its id |
| `send_prompt` | type into a live session's terminal; `submit: false` leaves it for the human |
| `session_events` | a session's witnessed ledger events, oldest first |
| `memory_search` | semantic search over memories and skills |
| `ledger_verify` | verify the ledger's hash chain |
| `decline` | decline (won't / shouldn't) or escalate (can't without a human) a task, with the reason and a suggestion; witnessed as `agent_declined`, shown in the Inbox as a decision, never retried by the console |
| `request_review` | ask the reviewer agent (a different identity: the other engine when installed, else the other model family) to adversarially review a session's finished turn; the verdict (`approve` / `request_changes` / `block`, with findings) is witnessed as `review_verdict` under `agent:reviewer:<engine>` and shown in the Inbox; a block stands until a human overrides it with a reason (`review_decision`) |

The allowlist for `invoke_action` is enforced on both sides
(`console_mcp::action_allowed` in Rust, `actionAllowed` in the webview host).
