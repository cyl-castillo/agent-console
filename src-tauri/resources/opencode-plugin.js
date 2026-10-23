// Agent Console witness for OpenCode — installed by the console into
// ~/.config/opencode/plugins/agent-console.js (idempotent, marker-guarded).
//
// OpenCode has no hooks, so without this a session of it leaves the ledger
// nothing per tool call. Inside an Agent Console terminal the CLI inherits
// AGENT_CONSOLE_SESSION_DIR / AGENT_CONSOLE_TERM_ID; the plugin appends the
// same event lines the hook-bridge writes for Claude and Codex to that
// directory's events.jsonl, and the console's watcher does the rest.
// Outside the console (no env, or the directory is gone) every hook is a
// no-op: the user's own OpenCode sessions are untouched. Nothing here runs a
// command, opens a socket or reads a file; it only appends JSON lines.
import { appendFileSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";

const EXCERPT_MAX = 1000;
const TAIL_MAX = 400;
const COMMAND_MAX = 500;
const SUMMARY_MAX = 1000;
const PROMPT_MAX = 4000;

const cap = (text, max) => {
  const chars = Array.from(text);
  return chars.length > max ? [chars.slice(0, max).join(""), true] : [text, false];
};
const tail = (text, headMax, tailMax) => {
  const chars = Array.from(text);
  if (chars.length <= headMax) return undefined;
  return chars.slice(Math.max(0, chars.length - tailMax)).join("");
};
const sha256 = (text) => createHash("sha256").update(text, "utf8").digest("hex");

export const AgentConsoleWitness = async ({ directory }) => {
  const dir = process.env.AGENT_CONSOLE_SESSION_DIR;
  const termId = process.env.AGENT_CONSOLE_TERM_ID;
  if (!dir || !existsSync(dir)) return {};
  const append = (event) => {
    try {
      appendFileSync(`${dir}/events.jsonl`, JSON.stringify(event) + "\n");
    } catch {
      // Never let a witness failure reach the agent.
    }
  };
  const base = (sessionID) => ({
    ts: Date.now(),
    ...(sessionID ? { sessionId: sessionID } : {}),
    ...(termId ? { termId } : {}),
    ...(directory ? { cwd: directory } : {}),
    observedVia: "plugin",
  });
  // The assistant's words of the turn: text parts keyed by part id, under
  // the message ids OpenCode reported as assistant messages.
  const assistantMessages = new Set();
  const texts = new Map(); // messageID -> Map(partID -> text)
  let lastAssistant = null;
  let turnOpen = false;

  return {
    "chat.message": async (input, output) => {
      const prompt = (output?.parts ?? [])
        .filter((p) => p && p.type === "text" && typeof p.text === "string")
        .map((p) => p.text)
        .join("\n")
        .trim();
      if (!prompt) return;
      const [text] = cap(prompt, PROMPT_MAX);
      turnOpen = true;
      assistantMessages.clear();
      texts.clear();
      lastAssistant = null;
      const e = { type: "user_prompt", ...base(input?.sessionID), prompt: text };
      if (input?.model?.providerID && input?.model?.modelID) {
        e.model = `${input.model.providerID}/${input.model.modelID}`;
      }
      append(e);
    },

    "tool.execute.after": async (input, output) => {
      const tool = input?.tool ?? "tool";
      const text = typeof output?.output === "string" ? output.output : "";
      const [excerpt, truncated] = cap(text, EXCERPT_MAX);
      const e = {
        type: "tool_result",
        ...base(input?.sessionID),
        tool,
        excerpt,
        truncated,
        outputSha256: sha256(text),
      };
      const t = tail(text, EXCERPT_MAX, TAIL_MAX);
      if (t !== undefined) e.excerptTail = t;
      if (input?.callID) e.toolUseId = input.callID;
      const command = input?.args?.command;
      if (typeof command === "string" && command) e.command = cap(command, COMMAND_MAX)[0];
      else if (Array.isArray(command) && command.length) {
        e.command = cap(command.join(" "), COMMAND_MAX)[0];
        e.commandArgv = command.filter((a) => typeof a === "string");
      }
      // The bash tool reports its exit code in metadata.exit (OpenCode 1.19).
      const exit = output?.metadata?.exit;
      if (typeof exit === "number") {
        e.exitCode = exit;
        e.exitCodeVia = "plugin";
        if (exit !== 0) e.failed = true;
      }
      append(e);
    },

    event: async ({ event }) => {
      const type = event?.type;
      const props = event?.properties ?? {};
      if (type === "message.updated" && props.info?.role === "assistant") {
        assistantMessages.add(props.info.id);
        lastAssistant = props.info.id;
        return;
      }
      if (type === "message.part.updated") {
        const part = props.part;
        if (part?.type === "text" && assistantMessages.has(part.messageID)) {
          if (!texts.has(part.messageID)) texts.set(part.messageID, new Map());
          texts.get(part.messageID).set(part.id, part.text ?? "");
        }
        return;
      }
      if (type === "session.idle") {
        if (!turnOpen) return;
        turnOpen = false;
        const e = { type: "turn_end", ...base(props.sessionID) };
        const words = lastAssistant && texts.has(lastAssistant)
          ? Array.from(texts.get(lastAssistant).values()).join("\n").trim()
          : "";
        if (words) {
          const [summary, truncated] = cap(words, SUMMARY_MAX);
          e.summary = summary;
          e.summaryTruncated = truncated;
        }
        append(e);
      }
    },
  };
};
