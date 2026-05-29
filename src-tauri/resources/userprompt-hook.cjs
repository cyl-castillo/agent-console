#!/usr/bin/env node
// Agent Console hook — UserPromptSubmit.
// Writes a JSONL event into the per-session events log, but only when the
// AGENT_CONSOLE_SESSION_DIR env var is set (i.e., claude is running inside
// the integrated terminal). Outside Agent Console, this is a silent no-op.

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  const prompt = input.user_prompt ?? input.prompt ?? input.message ?? "";
  const event = {
    type: "user_prompt",
    ts: Date.now(),
    prompt: typeof prompt === "string" ? prompt : "",
  };

  // Claude's hook payload carries the session id; surface it so the UI can
  // associate a terminal session with a resumable Claude conversation.
  const sid = input.session_id ?? input.sessionId;
  if (typeof sid === "string" && sid.length > 0) event.sessionId = sid;

  // The PTY that launched this claude tags itself via AGENT_CONSOLE_TERM_ID.
  // Carrying it back lets the UI bind the claude session id to the exact
  // terminal that emitted the prompt — instead of guessing "whatever is
  // active" (which misattributes when several claude sessions run at once).
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;

  // Detect a leading slash command — likely a skill or custom command invocation.
  if (event.prompt.startsWith("/")) {
    const m = event.prompt.match(/^\/([\w.-]+)/);
    if (m) event.skill = m[1];
  }

  try {
    fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n");
  } catch { /* ignore */ }
  process.exit(0);
});
