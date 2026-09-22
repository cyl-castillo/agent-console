#!/usr/bin/env node
// Agent Console hook — Notification observer (Claude).
//
// Claude says what it is waiting for: `permission_prompt` (a prompt has sat
// ~6 s), `idle_prompt` (~60 s idle after a turn), `agent_needs_input`,
// `agent_completed`, … Observer only — the CLI ignores our output — and a
// silent no-op outside Agent Console (no session dir).

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

const cap = (s, n) => (typeof s === "string" && s.length > n ? s.slice(0, n) : s);

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }
  const event = { type: "notification", ts: Date.now() };
  if (typeof input.notification_type === "string") event.notificationType = input.notification_type;
  if (typeof input.message === "string") event.message = cap(input.message, 500);
  if (typeof input.title === "string") event.title = cap(input.title, 200);
  if (typeof input.session_id === "string" && input.session_id.length > 0) event.sessionId = input.session_id;
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;
  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;
  try { fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n"); } catch { /* ignore */ }
  process.exit(0);
});
