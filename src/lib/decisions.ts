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

// One simple command whose head is here. Code runners (`node -e`,
// `python -c`) are not: they are shells with another syntax. Mirrors
// `remote_decisions.rs` (READ_ONLY / FIND_ACTS / GIT_BRANCH_*).
const READ_ONLY_HEAD =
  /^\s*(ls|pwd|cat|head|tail|wc|echo|which|type|printenv|date|whoami|stat|file|du|df|tree|rg|grep|egrep|fgrep|find(?![^|]*\s-(delete|exec|execdir|ok|okdir|fprint0?|fprintf|fls)\b)|git\s+(status|log|diff|show|remote\s+-v|rev-parse|describe|blame|stash\s+list)|npm\s+(test|run\s+test|run\s+lint|run\s+build|ls|outdated|view)|npx\s+(tsc|vitest|eslint|prettier\s+--check|playwright\s+test)|cargo\s+(check|test|clippy|fmt\s+--check|tree|metadata)|make\s+test|pytest|go\s+(test|vet|build))(?:\s|$)/;
// `git branch` only when it lists: no flag that deletes, moves, copies or
// sets upstream, and no bare name (which creates). Mirrors `GIT_BRANCH_*`.
const GIT_BRANCH_HEAD = /^\s*git\s+branch(?:\s|$)/;
const GIT_BRANCH_WRITES =
  /\s(?:-[a-zA-Z]*[dDfFmMcCu][a-zA-Z]*|--(?:delete|force|move|copy|set-upstream-to(?:=\S*)?|unset-upstream|edit-description|track|no-track))(?:\s|$)/;
const GIT_BRANCH_LISTS =
  /\s(?:--list|--contains|--no-contains|--merged|--no-merged|--points-at|--show-current|-[a-zA-Z]*l[a-zA-Z]*)(?:\s|=|$)/;
const GIT_BRANCH_POSITIONAL = /^\s*git\s+branch(?:\s+-\S+)*\s+[^-\s]/;
// Anything that chains, redirects, substitutes or backgrounds is not "one
// read-only command", whatever its head says.
const COMPOUND = /[;&>`\n]|\$\(|<<|<\(|\|\|/;
const XARGS = /\bxargs\b/;
// Options that turn a reader into a runner or a writer (`rg --pre <cmd>`,
// `git diff --output=<file>`, `--ext-diff`, `tree -o`). `env` and `less`
// left the allowlist for the same reason. Mirrors `READER_ESCAPES` in Rust.
const READER_ESCAPES =
  /^\s*rg\b.*\s--(pre|pre-glob|hostname-bin)\b|^\s*git\b.*\s--(output|ext-diff|textconv)\b|^\s*git\b.*\s-O\S|^\s*tree\b.*\s-[a-zA-Z]*o\b|^\s*file\b.*\s-[a-zA-Z]*C/;

function readOnlyHead(seg: string): boolean {
  if (READER_ESCAPES.test(seg)) return false;
  if (GIT_BRANCH_HEAD.test(seg)) {
    return (
      !GIT_BRANCH_WRITES.test(seg) &&
      (!GIT_BRANCH_POSITIONAL.test(seg) || GIT_BRANCH_LISTS.test(seg))
    );
  }
  return READ_ONLY_HEAD.test(seg);
}

/// Read-only = no compound syntax, no xargs, and every pipe segment's head in
/// the allowlist (`git log | head` is fine; `ls | xargs rm` is not).
export function isReadOnlyCommand(cmd: string): boolean {
  if (COMPOUND.test(cmd) || XARGS.test(cmd)) return false;
  const segments = cmd.split("|");
  return segments.length > 0 && segments.every((seg) => readOnlyHead(seg.trim()));
}

/// Paths where a write runs code later (hooks, shells, keys) or holds
/// secrets — high whatever the tool. Mirrors `SENSITIVE_PATH` in Rust.
const SENSITIVE_PATH =
  /(^|\/)(\.ssh|\.claude|\.codex|\.config\/opencode|\.git\/hooks|\.git\/config|\.gnupg|\.aws|\.kube|\.docker|\.npmrc|\.netrc|\.bashrc|\.zshrc|\.profile|\.bash_profile|\.zprofile|\.env(\.[^/]+)?)(\/|$)|^\/etc\/|\.(pem|key|p12|pfx)$/i;
const MCP_DESTRUCTIVE = /(delete|remove|destroy|drop|purge|send|publish|deploy|merge|push|force)/i;

/// The engines name the shell differently (Claude `Bash`, Codex `shell`,
/// OpenCode `bash`); one shape before classifying. Mirrors `normalize_tool`.
export function normalizeTool(tool: string): string {
  switch (tool) {
    case "Bash":
    case "bash":
    case "shell":
    case "local_shell":
    case "LocalShell":
    case "commandExecution":
    case "exec_command":
      return "Bash";
    case "edit":
      return "Edit";
    case "write":
      return "Write";
    case "multiedit":
      return "MultiEdit";
    case "read":
      return "Read";
    default:
      return tool;
  }
}

/// The command string of a shell request, whatever its shape.
export function commandOf(input: Record<string, unknown>): string {
  const c = input.command ?? input.cmd;
  if (typeof c === "string") return c;
  if (Array.isArray(c)) return c.filter((x) => typeof x === "string").join(" ");
  return "";
}

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
  const tool = normalizeTool(req.tool);
  if (tool === "Bash") {
    const cmd = commandOf(inp);
    const a = assessCommand(cmd);
    if (a?.level === "dangerous") return { tier: "high", label: "Irreversible", reason: a.reason };
    if (a) {
      return HIGH_REASON.test(a.reason)
        ? { tier: "high", label: "Irreversible or external", reason: a.reason }
        : { tier: "medium", label: "Changes the machine", reason: a.reason };
    }
    if (isReadOnlyCommand(cmd))
      return { tier: "low", label: "Read-only", reason: "inspects, builds or tests" };
    return { tier: "medium", label: "Runs a command", reason: "not in the read-only allowlist" };
  }
  if (tool === "Edit" || tool === "Write" || tool === "MultiEdit" || tool === "NotebookEdit") {
    const path =
      typeof inp.file_path === "string"
        ? inp.file_path
        : typeof inp.path === "string"
          ? inp.path
          : typeof inp.notebook_path === "string"
            ? inp.notebook_path
            : "";
    const rel = path ? toRelative(path, req.cwd) : null;
    const cwd = req.cwd.endsWith("/") ? req.cwd.slice(0, -1) : req.cwd;
    if (SENSITIVE_PATH.test(path) || SENSITIVE_PATH.test(rel ?? ""))
      return {
        tier: "high",
        label: "Touches secrets, hooks or a shell profile",
        reason: rel ?? path,
      };
    const outside = path.startsWith("/") && path !== cwd && !path.startsWith(`${cwd}/`);
    if (outside) return { tier: "high", label: "Writes outside the project", reason: path };
    if (/(^|\/)\.git\//.test(rel ?? path))
      return { tier: "high", label: "Touches secrets or git internals", reason: rel ?? path };
    if (/(^|\/)(tests?|__tests__|spec)\/|\.(test|spec)\.[cm]?[jt]sx?$/.test(rel ?? path))
      return {
        tier: "medium",
        label: "Edits a test",
        reason: "test files are the evidence; review the diff",
      };
    return { tier: "medium", label: "Edits a file", reason: rel ?? path };
  }
  if (["Read", "Glob", "Grep", "LS", "TodoWrite", "Task", "TaskOutput"].includes(tool)) {
    return { tier: "low", label: "Read-only", reason: tool };
  }
  // Same tier as `remote_decisions.rs`: leaving the machine is never "read-only".
  if (tool === "WebFetch" || tool === "WebSearch")
    return { tier: "medium", label: "Reaches a host", reason: "outside the machine" };
  if (tool.startsWith("mcp__"))
    return MCP_DESTRUCTIVE.test(tool)
      ? { tier: "high", label: "Tool server: destructive or external", reason: tool }
      : { tier: "medium", label: "Calls a tool server", reason: tool };
  return { tier: "medium", label: "Uses a tool", reason: tool };
}

/// What the user must type to approve a high-tier decision: the word, not a
/// click, is the friction (habit cannot do it for them).
export const CONFIRM_WORD = "approve";
