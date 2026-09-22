#!/usr/bin/env node
// Agent Console hook — PostToolUseFailure observer (Claude).
//
// A tool that ran and FAILED (a test suite exiting non-zero lands here, not
// in PostToolUse). Same correlation fields as a result; `error` is the text
// Claude saw and the exit code is parsed from its documented "Exit code N"
// first line. Silent no-op outside Agent Console.

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const EXCERPT_MAX = 1000;
const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }
  const event = { type: "tool_failed", ts: Date.now() };
  if (typeof input.tool_name === "string") event.tool = input.tool_name;
  const cmd = input.tool_input && typeof input.tool_input.command === "string" ? input.tool_input.command : "";
  if (cmd) event.command = cmd.length > 500 ? cmd.slice(0, 500) : cmd;
  if (typeof input.error === "string") {
    event.excerpt = input.error.length > EXCERPT_MAX ? input.error.slice(0, EXCERPT_MAX) : input.error;
    event.truncated = input.error.length > EXCERPT_MAX;
    event.outputSha256 = crypto.createHash("sha256").update(input.error).digest("hex");
    const m = /^Exit code (\d+)/.exec(input.error.split("\n")[0].trim());
    if (m) event.exitCode = parseInt(m[1], 10);
  }
  if (typeof input.is_interrupt === "boolean") event.interrupted = input.is_interrupt;
  if (typeof input.duration_ms === "number") event.durationMs = input.duration_ms;
  if (typeof input.session_id === "string" && input.session_id.length > 0) event.sessionId = input.session_id;
  if (typeof input.tool_use_id === "string" && input.tool_use_id.length > 0) event.toolUseId = input.tool_use_id;
  if (typeof input.agent_id === "string" && input.agent_id.length > 0) event.agentId = input.agent_id;
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;
  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;
  try { fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n"); } catch { /* ignore */ }
  process.exit(0);
});
