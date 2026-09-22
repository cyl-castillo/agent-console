#!/usr/bin/env node
// Agent Console hook — PermissionRequest bridge (Claude 2.1.x).
//
// Same file protocol as the PreToolUse bridge, but this event only fires
// when Claude was about to ASK the human — tools its rules already allow
// never reach here. Output uses PermissionRequest's nested `decision`
// object (exit code 2 is not honored for this event).
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
    source: "permission_request",
    // How long this hook will wait before falling back to the terminal
    // prompt — lets the UI show an honest countdown instead of a silent
    // stall (MEJORAS-2026-07 R2.8).
    timeoutMs: TIMEOUT_MS,
  };
  // The PTY that launched this claude tags itself via AGENT_CONSOLE_TERM_ID
  // (same binding the userprompt hook uses), so the UI can mark exactly which
  // session is blocked waiting on this approval.
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (typeof termId === "string" && termId.length > 0) req.termId = termId;
  if (typeof input.permission_mode === "string") req.permissionMode = input.permission_mode;
  if (Array.isArray(input.permission_suggestions) && input.permission_suggestions.length <= 16) {
    req.permissionSuggestions = input.permission_suggestions;
  }
  if (typeof input.session_id === "string" && input.session_id.length > 0) req.sessionId = input.session_id;

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

  // Real synchronous sleep between polls: Atomics.wait blocks this thread
  // without burning CPU. The previous empty-while spin pinned a full core for
  // the entire approval wait (up to 90s per tool call).
  const sleepBuf = new Int32Array(new SharedArrayBuffer(4));
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
    Atomics.wait(sleepBuf, 0, 0, POLL_MS);
  }

  // Cleanup
  try { fs.unlinkSync(reqPath); } catch { /* ignore */ }
  try { fs.unlinkSync(resPath); } catch { /* ignore */ }

  // No decision reached us in time: the CLI decides (its own prompt, or
  // auto-deny where it can't prompt). Leave a line so the ledger closes the
  // request as decided-outside instead of leaving it open forever.
  if (!decision || !["allow", "deny"].includes(decision)) {
    const deferred = { type: "approval_deferred", ts: Date.now(), approvalId: id, tool: req.tool };
    if (req.termId) deferred.termId = req.termId;
    try { fs.appendFileSync(path.join(sessionDir, "events.jsonl"), JSON.stringify(deferred) + "\n"); } catch { /* ignore */ }
  }

  // "ask" (or timeout/garbage) ⇒ `{}`: Claude shows its own prompt, or
  // auto-denies where it can't prompt (its documented default, not ours).
  if (!decision || !["allow", "deny"].includes(decision)) {
    process.stdout.write("{}");
    process.exit(0);
  }

  const out = {
    hookSpecificOutput: {
      hookEventName: "PermissionRequest",
      decision:
        decision === "allow"
          ? { behavior: "allow" }
          : { behavior: "deny", message: reason || "denied in the Agent Console approval modal" },
    },
  };
  process.stdout.write(JSON.stringify(out));
  process.exit(0);
});
