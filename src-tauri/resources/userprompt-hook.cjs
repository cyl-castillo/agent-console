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
//    The same answer may carry a sessionTitle (the name Agent Console shows
//    for this session): echoed back as hookSpecificOutput.sessionTitle so the
//    CLI's own session list matches the sidebar. The app only sends it when it
//    has a real name to give, and only once per change — a title typed inside
//    the CLI is never overwritten on every prompt. Older CLIs ignore the field.

const fs = require("fs");
const os = require("os");
const path = require("path");
const http = require("http");

// The prompt must never wait on us longer than this. The request resolves as
// soon as the server answers (typically ~1.1-1.5s of warm semantic search on
// modest CPUs), so this cap only bites when the answer would be lost anyway —
// a tight cap was costing the whole injection, not saving time.
const INJECT_TIMEOUT_MS = 2500;
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

// Nothing to add to this turn — what every miss collapses to.
const EMPTY = { context: null, sessionTitle: null };

function str(v) { return typeof v === "string" && v.length > 0 ? v : null; }

// POST the prompt to the app; call done({context, sessionTitle}) exactly once.
function fetchInjection(prompt, cwd, done) {
  const port = injectPort();
  if (!port) { done(EMPTY); return; }
  let finished = false;
  const finish = (res) => { if (!finished) { finished = true; done(res || EMPTY); } };
  // Outer guard: covers connect + response + parsing, whatever stalls.
  const guard = setTimeout(() => finish(EMPTY), INJECT_TIMEOUT_MS);
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
            const answer = JSON.parse(Buffer.concat(out).toString());
            finish({ context: str(answer.context), sessionTitle: str(answer.sessionTitle) });
          } catch { finish(EMPTY); }
        });
        res.on("error", () => finish(EMPTY));
      }
    );
    req.on("timeout", () => { req.destroy(); finish(EMPTY); });
    req.on("error", () => finish(EMPTY));
    req.end(body);
  } catch { finish(EMPTY); }
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

  fetchInjection(event.prompt, event.cwd ?? "", (answer) => {
    // Same schema for both engines (Codex adopted Claude's hook output).
    const out = { hookEventName: "UserPromptSubmit" };
    if (answer.context) out.additionalContext = answer.context;
    if (answer.sessionTitle) out.sessionTitle = answer.sessionTitle;
    // Nothing to say → say nothing at all: an empty hookSpecificOutput is
    // still output the CLI has to parse.
    if (out.additionalContext || out.sessionTitle) {
      process.stdout.write(JSON.stringify({ hookSpecificOutput: out }));
    }
    process.exit(0);
  });
});
