#!/usr/bin/env node
// Agent Console hook — PostToolUse (tool finished).
// Observer only: writes a `tool_result` event into the per-session events log
// so the Testigo ledger can record what each tool call produced inside a turn.
// It never blocks or alters the tool call. Only active when
// AGENT_CONSOLE_SESSION_DIR is set (agent running inside the integrated
// terminal); outside Agent Console it's a silent no-op.

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

// Tool responses can be huge (file reads, command output). The ledger only
// needs a bounded excerpt as evidence; cap it here so events.jsonl and the
// downstream ledger never balloon.
const EXCERPT_MAX = 1000;

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  const event = { type: "tool_result", ts: Date.now() };

  const tool = input.tool_name ?? input.toolName;
  if (typeof tool === "string" && tool.length > 0) event.tool = tool;

  let resp = input.tool_response ?? input.toolResponse;
  if (resp !== undefined) {
    let text;
    try { text = typeof resp === "string" ? resp : JSON.stringify(resp); } catch { text = String(resp); }
    if (typeof text === "string") {
      event.excerpt = text.length > EXCERPT_MAX ? text.slice(0, EXCERPT_MAX) : text;
      event.truncated = text.length > EXCERPT_MAX;
    }
  }

  const sid = input.session_id ?? input.sessionId;
  if (typeof sid === "string" && sid.length > 0) event.sessionId = sid;

  // Same terminal binding as the other hooks: attributes the result to the
  // exact session (and thus the open Testigo turn), not "whatever is active".
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;

  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;

  try {
    fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n");
  } catch { /* ignore */ }
  process.exit(0);
});
