#!/usr/bin/env node
// Agent Console PreToolUse hook.
// Reads the tool_use payload on stdin, asks Agent Console to approve dangerous
// tools (Bash/Edit/Write/MultiEdit/NotebookEdit), auto-approves safe ones.
//
// Coordination with the app happens via a shared directory pointed to by
// AGENT_CONSOLE_HOOK_DIR. The app writes res-<id>.json when the user decides.

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const SAFE_TOOLS = new Set([
  "Read", "Glob", "Grep", "LS",
  "WebFetch", "WebSearch",
  "TodoWrite", "Task",
  "ExitPlanMode",
]);

const TIMEOUT_MS = 120_000;
const POLL_INTERVAL_MS = 80;

// "allow":  exit 0 silently — the agent proceeds with this tool.
// "deny":   write a "block" decision so just this tool is rejected
//           (the agent can still try a different approach).
function allow()  { /* default behavior; no output needed */ }
function deny(r)  { process.stdout.write(JSON.stringify({ decision: "block", reason: r }) + "\n"); }

(async () => {
  const chunks = [];
  for await (const c of process.stdin) chunks.push(c);
  const raw = Buffer.concat(chunks).toString("utf8");

  let input = {};
  try { input = JSON.parse(raw); } catch { allow(); return; }

  const tool = input.tool_name ?? "";
  if (SAFE_TOOLS.has(tool)) { allow(); return; }

  const dir = process.env.AGENT_CONSOLE_HOOK_DIR;
  if (!dir || !fs.existsSync(dir)) {
    // Misconfiguration — fail-closed to avoid surprise side effects.
    deny("AGENT_CONSOLE_HOOK_DIR not set; cannot ask user");
    return;
  }

  // If the session was started with auto-approve, the app drops a sentinel.
  if (fs.existsSync(path.join(dir, "approve-all"))) { allow(); return; }

  const id = crypto.randomBytes(8).toString("hex");
  const reqFile = path.join(dir, `req-${id}.json`);
  const resFile = path.join(dir, `res-${id}.json`);

  try {
    fs.writeFileSync(reqFile, JSON.stringify({ id, tool_name: tool, tool_input: input.tool_input ?? {} }));
  } catch (e) {
    deny(`hook: cannot write request: ${e.message}`);
    return;
  }

  const deadline = Date.now() + TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (fs.existsSync(resFile)) {
      let decision = { allow: false };
      try { decision = JSON.parse(fs.readFileSync(resFile, "utf8")); } catch {}
      try { fs.unlinkSync(resFile); } catch {}
      try { fs.unlinkSync(reqFile); } catch {}
      if (decision.allow) allow();
      else deny(decision.reason || "User denied this action");
      return;
    }
    await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
  }
  try { fs.unlinkSync(reqFile); } catch {}
  deny("Approval timed out");
})();
