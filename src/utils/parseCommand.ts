import type { AgentMode } from "../types/domain";

export type ParsedCommand =
  | { kind: "noop" }
  | { kind: "send"; mode?: AgentMode; body: string }
  | { kind: "reset" }
  | { kind: "status" }
  | { kind: "help" };

const MODE_COMMANDS: Record<string, AgentMode> = {
  plan: "plan",
  build: "build",
  debug: "debug",
  review: "review",
};

/// Parse a single line of console input. Returns one of:
///   noop   → empty or just whitespace
///   reset  → /reset or /clear
///   status → /status
///   help   → /help
///   send   → text to dispatch to the agent (with optional mode override)
export function parseCommand(text: string): ParsedCommand {
  const t = text.trim();
  if (!t) return { kind: "noop" };
  if (!t.startsWith("/")) return { kind: "send", body: t };

  const space = t.indexOf(" ");
  const cmd = (space === -1 ? t : t.slice(0, space)).slice(1).toLowerCase();
  const rest = space === -1 ? "" : t.slice(space + 1).trim();

  if (cmd in MODE_COMMANDS) {
    if (!rest) return { kind: "noop" }; // mode change alone is a no-op; user uses selector for that
    return { kind: "send", mode: MODE_COMMANDS[cmd], body: rest };
  }
  switch (cmd) {
    case "reset":
    case "clear":  return { kind: "reset" };
    case "status": return { kind: "status" };
    case "help":   return { kind: "help" };
    default:       return { kind: "send", body: t }; // unknown slash → send as-is
  }
}

export const HELP_TEXT = [
  "Console commands:",
  "  /plan <text>    Analyze only, no edits or commands",
  "  /build <text>   Implement changes (default)",
  "  /debug <text>   Diagnose root cause before fixing",
  "  /review <text>  Review current diff, no modifications",
  "  /status         Show current session summary",
  "  /reset          Reset the session (clears memory)",
  "  /help           Show this help",
  "",
  "Plain text → dispatched with the active mode (selector below).",
  "Add constraints as chips; they're appended to every send.",
].join("\n");
