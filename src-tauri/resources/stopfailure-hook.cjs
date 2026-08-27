#!/usr/bin/env node
// Agent Console hook — StopFailure (turn died on an API error).
// Claude Code 2.1.78+ fires this INSTEAD of Stop when a turn ends because the
// API refused it: expired login, rate limit, billing, an overloaded model. The
// turn therefore never closes through the Stop bridge, and until now the app
// had no structured signal at all — the reason scrolled past in the terminal
// and the status pill sat "working" until the decay window gave up.
//
// This writes a `turn_failed` event so the app can close the turn honestly and
// say WHY: Claude ships the reason as a small enum (`error`) plus free-text
// `error_details`. Only active when AGENT_CONSOLE_SESSION_DIR is set (i.e. the
// agent runs inside the integrated terminal); outside Agent Console it's a
// silent no-op.

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

// The CLI's free-text detail can carry a whole API error body; the ledger only
// needs enough to read what went wrong. Same cap as the other bridges.
const DETAILS_MAX = 1000;
const SUMMARY_MAX = 1000;

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  const event = { type: "turn_failed", ts: Date.now() };

  // The reason, as one of the CLI's documented error kinds:
  // authentication_failed | oauth_org_not_allowed | account_on_hold |
  // billing_error | rate_limit | overloaded | invalid_request |
  // model_not_found | server_error | max_output_tokens | unknown.
  // We pass it through verbatim rather than mapping it: the app classifies,
  // and an enum value we don't know yet must still reach the UI.
  const err = input.error ?? input.errorType;
  if (typeof err === "string" && err.length > 0) event.error = err;

  const details = input.error_details ?? input.errorDetails;
  if (typeof details === "string" && details.trim().length > 0) {
    const text = details.trim();
    event.errorDetails = text.length > DETAILS_MAX ? text.slice(0, DETAILS_MAX) : text;
    event.errorDetailsTruncated = text.length > DETAILS_MAX;
  }

  const sid = input.session_id ?? input.sessionId;
  if (typeof sid === "string" && sid.length > 0) event.sessionId = sid;

  // Same terminal binding as the other bridges: the failure belongs to one
  // session, not to "whatever is active".
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;

  // Where the turn ran, so the post-turn snapshot captures the right checkout
  // for worktree sessions (a turn can change files before the API refuses it).
  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;

  // StopFailure carries the same `last_assistant_message` as Stop when the turn
  // said anything before dying.
  const last = input.last_assistant_message ?? input.lastAssistantMessage;
  if (typeof last === "string" && last.trim().length > 0) {
    const text = last.trim();
    event.summary = text.length > SUMMARY_MAX ? text.slice(0, SUMMARY_MAX) : text;
    event.summaryTruncated = text.length > SUMMARY_MAX;
  }

  try {
    fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n");
  } catch { /* ignore */ }
  process.exit(0);
});
