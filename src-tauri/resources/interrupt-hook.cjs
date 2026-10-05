#!/usr/bin/env node
// Agent Console hook — Interrupt (turn cut short by the user).
// Codex 0.150+ fires this INSTEAD of Stop when a top-level turn is interrupted
// (Esc / Ctrl-C in the TUI): the turn is over, but through a door that never
// reaches the Stop bridge. Until now such a turn stayed open in the ledger and
// the status pill sat "working" until the decay window gave up — the Codex
// twin of what StopFailure fixes on the Claude side.
//
// This writes a `turn_interrupted` event so the app closes the turn honestly
// (with the diff of whatever the turn had changed by then) and marks it as cut
// short rather than finished. Codex sends no `last_assistant_message` here, so
// unlike Stop there are no closing words to carry. Only active when
// AGENT_CONSOLE_SESSION_DIR is set (i.e. the agent runs inside the integrated
// terminal); outside Agent Console it's a silent no-op.

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  const event = { type: "turn_interrupted", ts: Date.now() };

  const sid = input.session_id ?? input.sessionId;
  if (typeof sid === "string" && sid.length > 0) event.sessionId = sid;

  // Same terminal binding as the other bridges: the interrupted turn belongs
  // to one session, not to "whatever is active".
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;

  // Where the turn ran, so the post-turn snapshot captures the right checkout
  // for worktree sessions (a turn can change files before it's cut short).
  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;

  try {
    fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n");
  } catch { /* ignore */ }
  process.exit(0);
});
