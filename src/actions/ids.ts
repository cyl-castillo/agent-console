/// The console's action contract: every id here is stable — the palette,
/// voice commands, the agent bridge (window.__agentConsole) and, next, the
/// console's MCP server address actions by these names. Adding an id is a
/// documented change (docs/ACTIONS.md); renaming or removing one is a
/// breaking change for every client. Tested against the palette registry.
export const ACTION_IDS = [
  // Navigation
  "nav.terminal",
  "nav.changes",
  "nav.preview",
  "nav.skills",
  "nav.permissions",
  "nav.advisor",
  "nav.vault",
  "nav.context",
  "nav.plugins",
  "nav.mcp",
  "nav.feedback",
  "nav.tasks",
  "nav.teams",
  "nav.agenda",
  "nav.notes",
  "nav.proof",
  "nav.learning",
  "nav.roundtable",
  "nav.jobs",
  "nav.schedule",
  "nav.transfer",
  "modules.configure",
  // Git
  "git.commit",
  "git.push",
  "git.open_pr",
  "git.refresh",
  // Sessions and prompts
  "session.new",
  "session.close",
  "prompt.rerun_last",
  "prompt.composer",
  "snapshot.restore_latest",
  "snapshot.undo_restore",
  // Shell and app
  "ui.toggle_sidebar",
  "ui.toggle_right_panel",
  "ui.toggle_theme",
  "project.copy_path",
  "auth.claude_login",
  "auth.codex_login",
  "app.check_updates",
  "app.report_problem",
  "setup.wizard",
  "help.getting_started",
] as const;

export type ActionId = (typeof ACTION_IDS)[number];
