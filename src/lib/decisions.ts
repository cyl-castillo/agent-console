import type { ApprovalRequest } from "../types/domain";
import { assessCommand, toRelative } from "../permissions/rules";
import { hostsOutside } from "./perimeter";

/// Friction proportional to reversibility and external reach (UI-2026-10.md
/// §Principios 3): low = read-only or trivially undone, medium = edits inside
/// the project / builds / tests, high = irreversible or leaves the machine.
/// A high-tier decision requires a typed confirmation; a low one needs none.
export type DecisionTier = "low" | "medium" | "high";

export interface TierInfo {
  tier: DecisionTier;
  label: string;
  reason: string;
}

const READ_ONLY_BASH =
  /^\s*(ls|pwd|cat|head|tail|less|wc|echo|which|type|env|printenv|date|whoami|stat|file|du|df|tree|rg|grep|egrep|fgrep|find(?![^|]*-delete)|git\s+(status|log|diff|show|branch(?!\s+-[a-zA-Z]*[dD])|remote\s+-v|rev-parse|describe|blame|stash\s+list)|npm\s+(test|run\s+test|run\s+lint|run\s+build|ls|outdated|view)|npx\s+(tsc|vitest|eslint|prettier\s+--check|playwright\s+test)|cargo\s+(check|test|clippy|fmt\s+--check|tree|metadata)|node\s+-e|python3?\s+-c|make\s+test|pytest|go\s+(test|vet|build))\b/;

const HIGH_REASON = /remote|delete|discards|destroys|removes|elevated|truncates|rewrites/i;

export interface Perimeter {
  /// The session's declared allowed hosts; undefined/empty = none declared.
  allowedHosts?: string[];
}

export function tierFor(req: ApprovalRequest, perimeter?: Perimeter): TierInfo {
  // D14: reaching outside the session's declared perimeter is always high
  // tier, whatever the tool — the one gate the console actually controls.
  const outside = hostsOutside(req, perimeter?.allowedHosts);
  if (outside.length > 0) {
    return {
      tier: "high",
      label: "Outside the session perimeter",
      reason: `${outside.join(", ")} is not an allowed host of this session`,
    };
  }
  const inp = req.input ?? {};
  const tool = req.tool;
  if (tool === "Bash") {
    const cmd = typeof inp.command === "string" ? inp.command : "";
    const a = assessCommand(cmd);
    if (a?.level === "dangerous") return { tier: "high", label: "Irreversible", reason: a.reason };
    if (a) {
      return HIGH_REASON.test(a.reason)
        ? { tier: "high", label: "Irreversible or external", reason: a.reason }
        : { tier: "medium", label: "Changes the machine", reason: a.reason };
    }
    if (READ_ONLY_BASH.test(cmd))
      return { tier: "low", label: "Read-only", reason: "inspects, builds or tests" };
    return { tier: "medium", label: "Runs a command", reason: "not in the read-only allowlist" };
  }
  if (tool === "Edit" || tool === "Write" || tool === "MultiEdit" || tool === "NotebookEdit") {
    const path = typeof inp.file_path === "string" ? inp.file_path : "";
    const rel = path ? toRelative(path, req.cwd) : null;
    const cwd = req.cwd.endsWith("/") ? req.cwd.slice(0, -1) : req.cwd;
    const outside = path.startsWith("/") && path !== cwd && !path.startsWith(`${cwd}/`);
    if (outside) return { tier: "high", label: "Writes outside the project", reason: path };
    if (/(^|\/)(\.env|.*\.(pem|key|p12))$|(^|\/)\.git\//.test(rel ?? path))
      return { tier: "high", label: "Touches secrets or git internals", reason: rel ?? path };
    if (/(^|\/)(tests?|__tests__|spec)\/|\.(test|spec)\.[cm]?[jt]sx?$/.test(rel ?? path))
      return {
        tier: "medium",
        label: "Edits a test",
        reason: "test files are the evidence; review the diff",
      };
    return { tier: "medium", label: "Edits a file", reason: rel ?? path };
  }
  if (
    [
      "Read",
      "Glob",
      "Grep",
      "LS",
      "WebSearch",
      "WebFetch",
      "TodoWrite",
      "Task",
      "TaskOutput",
    ].includes(tool)
  ) {
    return { tier: "low", label: "Read-only", reason: tool };
  }
  if (tool.startsWith("mcp__"))
    return { tier: "medium", label: "Calls a tool server", reason: tool };
  return { tier: "medium", label: "Uses a tool", reason: tool };
}

/// What the user must type to approve a high-tier decision: the word, not a
/// click, is the friction (habit cannot do it for them).
export const CONFIRM_WORD = "approve";
