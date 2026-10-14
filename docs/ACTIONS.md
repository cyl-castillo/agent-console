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
| `nav.terminal` | the Terminal tab (v2: the conversation) |
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
| `ui.toggle_sidebar` | show/hide the workspace sidebar (v2: the list pane) |
| `ui.toggle_right_panel` | show/hide the right panel |
| `ui.toggle_theme` | dark ↔ light |
| `ui.toggle_v2` | the redesigned shell (preview flag) |
| `project.copy_path` | copy the project path |
| `auth.claude_login` | open a `claude login` session |
| `auth.codex_login` | open a `codex login` session |
| `app.check_updates` | check for a new release |
| `app.report_problem` | open a prefilled GitHub issue |
| `setup.wizard` | the first-run wizard |
| `help.getting_started` | the Getting Started guide |

## The bridge

`window.__agentConsole` exposes `listActions()`, `invoke(id)` and `state()`
(project, sessions, pending decisions, destination, `ui.v2`). Reads come from
the stores; every write goes through an action. In the app it is reachable
from the console's own code and tests; the MCP server relays the same calls
from the loopback listener.
