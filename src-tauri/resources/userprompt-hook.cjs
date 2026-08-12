#!/usr/bin/env node
// Agent Console hook — UserPromptSubmit.
//
// Two jobs, both strictly best-effort:
// 1) Observer: append a JSONL event to the per-session events log, but only
//    when AGENT_CONSOLE_SESSION_DIR is set (i.e., the agent runs inside the
//    integrated terminal). Outside Agent Console, this is a silent no-op.
// 2) Memory injection (E1 of the knowledge flywheel): ask the app's
//    loopback-only inject endpoint for memories relevant to this prompt and
//    hand them back as hookSpecificOutput.additionalContext. A hard timeout
//    bounds the wait — the app being closed, busy, or gone must never delay
//    the prompt or surface an error to the CLI.

const fs = require("fs");
const os = require("os");
const path = require("path");
const http = require("http");

// The prompt must never wait on us longer than this.
const INJECT_TIMEOUT_MS = 1500;
// Below this length a prompt is a nudge ("ok", "dale") — not worth a search.
const MIN_PROMPT_CHARS = 12;

const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
if (!dir || !fs.existsSync(dir)) { process.exit(0); }

// Mirrors the Rust side's dirs::data_local_dir(), where inject-port.json lives.
function dataDir() {
  if (process.platform === "win32") return process.env.LOCALAPPDATA || null;
  if (process.platform === "darwin") return path.join(os.homedir(), "Library", "Application Support");
  return process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
}

function injectPort() {
  try {
    const base = dataDir();
    if (!base) return null;
    const raw = fs.readFileSync(path.join(base, "agent-console", "inject-port.json"), "utf8");
    const port = JSON.parse(raw).port;
    return Number.isInteger(port) && port > 0 && port < 65536 ? port : null;
  } catch { return null; }
}

// POST the prompt to the app; call done(contextOrNull) exactly once.
function fetchContext(prompt, cwd, done) {
  const port = injectPort();
  if (!port) { done(null); return; }
  let finished = false;
  const finish = (ctx) => { if (!finished) { finished = true; done(ctx); } };
  // Outer guard: covers connect + response + parsing, whatever stalls.
  const guard = setTimeout(() => finish(null), INJECT_TIMEOUT_MS);
  guard.unref?.();
  try {
    const body = JSON.stringify({ prompt, cwd, termId: process.env.AGENT_CONSOLE_TERM_ID || null });
    const req = http.request(
      { host: "127.0.0.1", port, path: "/inject", method: "POST",
        headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) },
        timeout: INJECT_TIMEOUT_MS },
      (res) => {
        let out = [];
        res.on("data", (c) => out.push(c));
        res.on("end", () => {
          try {
            const ctx = JSON.parse(Buffer.concat(out).toString()).context;
            finish(typeof ctx === "string" && ctx.length > 0 ? ctx : null);
          } catch { finish(null); }
        });
        res.on("error", () => finish(null));
      }
    );
    req.on("timeout", () => { req.destroy(); finish(null); });
    req.on("error", () => finish(null));
    req.end(body);
  } catch { finish(null); }
}

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

  // Where claude is actually running. Sessions in an isolated worktree have a
  // cwd different from the project root — the auto-snapshot must capture THAT
  // working tree, not the main checkout.
  if (typeof input.cwd === "string" && input.cwd.length > 0) event.cwd = input.cwd;

  // Detect a leading slash command — likely a skill or custom command invocation.
  if (event.prompt.startsWith("/")) {
    const m = event.prompt.match(/^\/([\w.-]+)/);
    if (m) event.skill = m[1];
  }

  try {
    fs.appendFileSync(path.join(dir, "events.jsonl"), JSON.stringify(event) + "\n");
  } catch { /* ignore */ }

  // Slash commands carry their own instructions; short prompts carry nothing
  // to search for. Both skip straight to a clean exit.
  const wantsInjection =
    event.prompt.length >= MIN_PROMPT_CHARS && !event.prompt.startsWith("/");
  if (!wantsInjection) { process.exit(0); }

  fetchContext(event.prompt, event.cwd ?? "", (ctx) => {
    if (ctx) {
      // Same schema for both engines (Codex adopted Claude's hook output).
      process.stdout.write(JSON.stringify({
        hookSpecificOutput: {
          hookEventName: "UserPromptSubmit",
          additionalContext: ctx,
        },
      }));
    }
    process.exit(0);
  });
});
