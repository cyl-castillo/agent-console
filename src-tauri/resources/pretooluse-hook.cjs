#!/usr/bin/env node
// Agent Console hook — PreToolUse bridge.
//
// Gated on two env vars set by Agent Console when it spawns `claude`:
//   AGENT_CONSOLE_BRIDGE=1
//   AGENT_CONSOLE_SESSION_DIR=/path/to/session
// Outside Agent Console (both vars unset) this is an immediate silent
// pass-through so user's regular `claude` sessions are not affected.
//
// Protocol:
//   1) Read Claude Code's PreToolUse JSON from stdin.
//   2) Write a request file to <session_dir>/approvals/<id>.req.json.
//   3) Poll <session_dir>/approvals/<id>.res.json (decision from UI).
//   4) On timeout, fall back to "ask" so the user is never silently bypassed.
//   5) Emit the decision JSON on stdout for Claude Code.

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const TIMEOUT_MS = parseInt(process.env.AGENT_CONSOLE_APPROVAL_TIMEOUT_MS || "90000", 10);
const POLL_MS = 80;

function passThrough() { process.exit(0); }

if (process.env.AGENT_CONSOLE_BRIDGE !== "1") { passThrough(); }
const sessionDir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!sessionDir || !fs.existsSync(sessionDir)) { passThrough(); }

const approvalsDir = path.join(sessionDir, "approvals");
try { fs.mkdirSync(approvalsDir, { recursive: true }); } catch { /* ignore */ }

let chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {
  let input = {};
  try { input = JSON.parse(Buffer.concat(chunks).toString()); } catch { /* ignore */ }

  const id = crypto.randomUUID();
  const req = {
    id,
    ts: Date.now(),
    sessionDir,
    cwd: input.cwd || process.cwd(),
    tool: input.tool_name || "Unknown",
    input: input.tool_input || {},
  };

  const reqPath = path.join(approvalsDir, `${id}.req.json`);
  const resPath = path.join(approvalsDir, `${id}.res.json`);

  try {
    fs.writeFileSync(reqPath, JSON.stringify(req));
  } catch (e) {
    // If we can't write the request, fail open to Claude's native prompt.
    passThrough();
  }

  const deadline = Date.now() + TIMEOUT_MS;
  let decision = null;
  let reason = null;

  while (Date.now() < deadline) {
    if (fs.existsSync(resPath)) {
      try {
        const txt = fs.readFileSync(resPath, "utf8");
        const res = JSON.parse(txt);
        decision = res.decision;
        reason = res.reason || null;
        break;
      } catch { /* keep polling — file may still be writing */ }
    }
    // Busy-sleep without blocking event loop indefinitely.
    const end = Date.now() + POLL_MS;
    while (Date.now() < end) { /* spin */ }
  }

  // Cleanup
  try { fs.unlinkSync(reqPath); } catch { /* ignore */ }
  try { fs.unlinkSync(resPath); } catch { /* ignore */ }

  if (!decision || !["allow", "deny", "ask"].includes(decision)) {
    decision = "ask";
    reason = reason || "Agent Console: no response within timeout — falling back to Claude's native prompt";
  }

  const out = {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: decision,
      permissionDecisionReason: reason || `agent-console approval modal: ${decision}`,
    },
  };
  process.stdout.write(JSON.stringify(out));
  process.exit(0);
});
