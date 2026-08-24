#!/usr/bin/env node
// Agent Console hook — Stop (turn completed).
// Both Claude Code and Codex fire this when the agent finishes a turn. It
// writes a `turn_end` event into the per-session events log so the UI can show
// a REAL "finished" signal instead of guessing from activity decay. Only active
// when AGENT_CONSOLE_SESSION_DIR is set (i.e., the agent runs inside the
// integrated terminal); outside Agent Console it's a silent no-op.

const fs = require("fs");
const path = require("path");

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

// The agent's closing words can be a whole essay. The ledger only needs enough
// to read what the turn claims it did; same cap as the tool-result excerpt so
// events.jsonl stays bounded no matter how chatty the turn was.
const SUMMARY_MAX = 1000;

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  const event = { type: "turn_end", ts: Date.now() };

  const sid = input.session_id ?? input.sessionId;
  if (typeof sid === "string" && sid.length > 0) event.sessionId = sid;

  // Same terminal binding as the prompt hook: lets the UI attribute the
  // finished turn to the exact session, not "whatever is active".
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) event.termId = termId;

  // Where the agent ran, so the post-turn snapshot (Testigo diff) captures the
  // right checkout for worktree sessions. The prompt hook stores it too; this
  // is the fallback when the turn state was lost (e.g. app restart mid-turn).
  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;

  // What the agent SAID it did, straight from the CLI (Claude 2.1.47+ ships
  // `last_assistant_message` in the Stop payload). Without it the ledger closes
  // a turn with a file diff and no words; with it the close carries the agent's
  // own claim next to the evidence. Absent on CLIs that don't send it (older
  // Claude, Codex today) — then the event is exactly what it was before.
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
