/// OSC 7501, the Program Status Protocol (Mitchell Hashimoto, Oct 2026):
/// a program tells its terminal whether it is idle, working, done, blocked
/// on the user or failed. Claude Code 2.1.295+ emits it once the terminal
/// answers its probe — `OSC 7501 ; ? ST` — with an OSC 7501 whose data
/// starts with `?`. Agent Console's terminal does, so the agent's state
/// arrives as a fact from the CLI, not as a guess from timers or hooks.
///
/// Claude's emitter (read from the binary, 2.1.307):
/// `state=<s>:app=claude-code[:id=…][:kind=…][:progress=N][:title=<b64>][:msg=<b64>]`
/// and `state=clear[:id=…]`.

export type ProgramState = "idle" | "working" | "blocked" | "done" | "error" | "clear";

export interface ProgramStatus {
  state: ProgramState;
  app?: string;
  id?: string;
  /// Why it is blocked (Claude: permission, question, …).
  kind?: string;
  progress?: number;
  title?: string;
  msg?: string;
}

const STATES: ReadonlySet<string> = new Set([
  "idle",
  "working",
  "blocked",
  "done",
  "error",
  "clear",
]);

/// The probe a program sends to ask whether the terminal speaks OSC 7501.
export const OSC_7501_PROBE = "?";
/// The answer that says yes (an OSC 7501 whose data starts with `?`).
export const OSC_7501_REPLY = "\x1b]7501;?\x1b\\";

function b64(text: string): string | undefined {
  try {
    const bin = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
    const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return undefined;
  }
}

/// Parse the data of an OSC 7501 sequence (everything after `7501;`).
/// Returns null for the probe and for anything that is not a status.
export function parseProgramStatus(data: string): ProgramStatus | null {
  if (data.startsWith("?")) return null;
  let state: string | undefined;
  const out: Partial<ProgramStatus> = {};
  for (const pair of data.split(":")) {
    const eq = pair.indexOf("=");
    if (eq <= 0) continue;
    const key = pair.slice(0, eq);
    const value = pair.slice(eq + 1);
    switch (key) {
      case "state":
        state = value;
        break;
      case "app":
        out.app = value;
        break;
      case "id":
        out.id = value;
        break;
      case "kind":
        out.kind = value;
        break;
      case "progress": {
        const n = Number(value);
        if (Number.isFinite(n)) out.progress = Math.min(100, Math.max(0, n));
        break;
      }
      case "title":
        out.title = b64(value);
        break;
      case "msg":
        out.msg = b64(value);
        break;
      default:
        break;
    }
  }
  if (!state || !STATES.has(state)) return null;
  return { state: state as ProgramState, ...out };
}
