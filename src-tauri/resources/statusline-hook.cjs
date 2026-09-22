#!/usr/bin/env node
// Agent Console — statusLine stand-in (node fallback for the native bridge).
//
// Runs on every status-line render, inside AND outside Agent Console. Inside
// (AGENT_CONSOLE_SESSION_DIR set): record the render — model, cost, context —
// as a `status` event for the app, deduped so vim-mode toggles don't spam.
// Always: chain to the user's ORIGINAL status line command (parked in
// <data dir>/agent-console/statusline-chain.json when ours was installed)
// with the same stdin, and pass its output through. No original ⇒ print
// nothing: they had no status line, they still don't.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

function dataDir() {
  if (process.platform === "win32") return process.env.LOCALAPPDATA || null;
  if (process.platform === "darwin") return path.join(os.homedir(), "Library", "Application Support");
  return process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
}

function statusEvent(input, termId, ts) {
  const e = { type: "status", ts };
  if (typeof input.session_id === "string") e.sessionId = input.session_id;
  if (termId) e.termId = termId;
  if (input.model) {
    if (typeof input.model.id === "string") e.modelId = input.model.id;
    if (typeof input.model.display_name === "string") e.modelName = input.model.display_name;
  }
  const c = input.cost || {};
  if (typeof c.total_cost_usd === "number") e.costUsd = c.total_cost_usd;
  if (typeof c.total_lines_added === "number") e.linesAdded = c.total_lines_added;
  if (typeof c.total_lines_removed === "number") e.linesRemoved = c.total_lines_removed;
  if (typeof c.total_duration_ms === "number") e.durationMs = c.total_duration_ms;
  const cw = input.context_window || {};
  if (typeof cw.context_window_size === "number") e.contextSize = cw.context_window_size;
  if (typeof cw.used_percentage === "number") e.usedPct = cw.used_percentage;
  if (cw.current_usage) {
    const u = cw.current_usage;
    const n = (k) => (typeof u[k] === "number" ? u[k] : 0);
    e.contextUsed = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    e.outputTokens = n("output_tokens");
  }
  if (typeof cw.total_input_tokens === "number") e.inputTotal = cw.total_input_tokens;
  if (typeof cw.total_output_tokens === "number") e.outputTotal = cw.total_output_tokens;
  if (typeof input.exceeds_200k_tokens === "boolean") e.exceeds200k = input.exceeds_200k_tokens;
  return e;
}

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  const raw = Buffer.concat(chunks);
  const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
  if (dir && fs.existsSync(dir)) {
    let input = {};
    try { input = JSON.parse(raw.toString()) || {}; } catch { /* ignore */ }
    const termId = process.env.AGENT_CONSOLE_TERM_ID || "";
    const event = statusEvent(input, termId, Date.now());
    const { ts: _ts, ...rest } = event;
    const fp = JSON.stringify(rest);
    const lastPath = path.join(dir, `status-last-${termId || "default"}.txt`);
    let unchanged = false;
    try { unchanged = fs.readFileSync(lastPath, "utf8") === fp; } catch { /* first render */ }
    if (!unchanged) {
      try { fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n"); } catch { /* ignore */ }
      try { fs.writeFileSync(lastPath, fp); } catch { /* ignore */ }
    }
  }
  // Chain to the original status line.
  try {
    const base = dataDir();
    if (!base) process.exit(0);
    const chain = JSON.parse(fs.readFileSync(path.join(base, "agent-console", "statusline-chain.json"), "utf8"));
    const cmd = typeof chain.command === "string" ? chain.command.trim() : "";
    if (!cmd) process.exit(0);
    const r = process.platform === "win32"
      ? spawnSync("cmd", ["/C", cmd], { input: raw, stdio: ["pipe", "pipe", "ignore"] })
      : spawnSync("/bin/sh", ["-c", cmd], { input: raw, stdio: ["pipe", "pipe", "ignore"] });
    if (r.stdout) process.stdout.write(r.stdout);
  } catch { /* no chain, or it failed: empty status line, never a broken session */ }
  process.exit(0);
});
