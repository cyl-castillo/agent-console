#!/usr/bin/env node
// Agent Console hook — PostModelSwitch (the session's model actually changed).
// Claude Code 2.1.251+ only; Codex has no equivalent event, so this bridge is
// never mirrored into ~/.codex/hooks.json. It writes a `model_switch` event so
// the console's model pill can show what the agent is REALLY running instead of
// the last thing we asked for — the two drift apart whenever the user types
// `/model` in the terminal, our own `/model` push lands while the agent is busy,
// or Claude restores/falls back to another model on its own. Async event: the
// CLI ignores our stdout, so this is a pure observer.
// Only active when AGENT_CONSOLE_SESSION_DIR is set (i.e., the agent runs inside
// the integrated terminal); outside Agent Console it's a silent no-op.

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

// Model names end up interpolated into the resume command (`claude --model
// <m>`), so the reader validates them; the cap here just keeps a hostile or
// buggy payload from bloating events.jsonl.
const MODEL_MAX = 128;

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  // A subagent switching its own model says nothing about the session the pill
  // describes — `agent_id` is only present inside one, so its presence is the
  // filter. Without this, a Task/subagent run would rewrite the session's model.
  const agentId = input.agent_id ?? input.agentId;
  if (typeof agentId === "string" && agentId.length > 0) { process.exit(0); }

  const to = input.to_model ?? input.toModel;
  if (typeof to !== "string" || to.trim().length === 0) { process.exit(0); }

  const event = { type: "model_switch", ts: Date.now(), model: to.trim().slice(0, MODEL_MAX) };

  const from = input.from_model ?? input.fromModel;
  if (typeof from === "string" && from.trim().length > 0) {
    event.fromModel = from.trim().slice(0, MODEL_MAX);
  }

  const sid = input.session_id ?? input.sessionId;
  if (typeof sid === "string" && sid.length > 0) event.sessionId = sid;

  // Same terminal binding as every other bridge: the switch belongs to ONE
  // session, not "whatever is active" when the event lands.
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;

  try {
    fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n");
  } catch { /* ignore */ }
  process.exit(0);
});
