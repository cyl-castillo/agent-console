// Pure functions: build raw rule strings, classify risk, suggest rules
// from a tool-use request, and screen against the hard-deny list.

import type {
  Effect, RiskLevel, RuleSuggestion, Scope, ToolUseRequest,
} from "./types";

const PATH_TOOLS = new Set(["Edit", "Write", "MultiEdit", "Read", "NotebookEdit"]);

// Patterns that are NEVER allowed, regardless of confirmation. Allow rules
// matching any of these are refused by the UI. Deny rules are fine.
const HARD_DENY_ALLOW_PATTERNS: Array<{ test: (raw: string) => boolean; reason: string }> = [
  { test: (r) => /^Bash\(rm\s+-rf?\s+[/*~]/.test(r), reason: "matches recursive delete from root or home" },
  { test: (r) => /^Bash\(sudo(\s|:)/.test(r) && !r.includes("AGENT_CONSOLE"), reason: "sudo cannot be auto-allowed" },
  { test: (r) => /^Bash\(:\(\)/.test(r), reason: "fork bomb pattern" },
  { test: (r) => /^Bash\(dd\s/.test(r), reason: "raw disk write" },
  { test: (r) => /^Bash\(mkfs/.test(r), reason: "filesystem format" },
  { test: (r) => /^(Write|Edit|MultiEdit)\(.*\.env/i.test(r), reason: "touches .env files" },
  { test: (r) => /^(Write|Edit|MultiEdit)\(.*\.ssh/i.test(r), reason: "touches SSH keys" },
  { test: (r) => /^(Write|Edit|MultiEdit)\(.*\.aws/i.test(r), reason: "touches AWS credentials" },
  { test: (r) => /^(Write|Edit|MultiEdit)\(.*\.git\//i.test(r), reason: "touches .git internals" },
  { test: (r) => /^(Write|Edit|MultiEdit)\(\/.*\)$/.test(r), reason: "absolute path outside cwd" },
];

// Bash commands that, when used with `:*` suffix in global scope, are broad enough
// to flag. In project scope they're acceptable.
const BROAD_GLOBAL_BASH_PREFIXES = new Set([
  "git", "npm", "pnpm", "yarn", "cargo", "make", "docker", "kubectl",
]);

// Live-command risk assessment: separate from HARD_DENY_ALLOW_PATTERNS, which
// classifies *allow rule patterns* for the permission gate. These classify
// *actual commands* for the single-approval badge. The two lists answer
// different questions and will legitimately diverge (e.g. `git push --force`
// must stay approvable as a rule, but still warrants a caution badge per-run).
// Keyboard-blocking tier: commands where the badge alone is insufficient because
// their effect is unknowable at a glance (curl|bash downloads arbitrary code) or
// directly targets hardware (block device writes). NOT placed here: rm -rf ./local,
// git push --force, reset --hard — those are predictable in effect and common
// enough that blocking the keyboard creates rubber-stamp-via-mouse behavior.
const DANGEROUS_COMMAND_PATTERNS: Array<{ re: RegExp; reason: string }> = [
  { re: /curl[^|#\n]*\|\s*(ba)?sh/i, reason: "downloads and executes remote code" },
  { re: /wget[^|#\n]*\|\s*(ba)?sh/i, reason: "downloads and executes remote code" },
  { re: />\s*\/dev\/(sda|hda|nvme|disk)\d*/i, reason: "raw block device write" },
  { re: /\bdd\s+if=/i, reason: "raw block device write" },
  { re: /\bmkfs\b/i, reason: "formats a filesystem" },
];

// Badge-only tier: amber chip, Ctrl+Enter still works. Broad enough to cover
// the common irreversible actions in agent sessions without adding mouse-click
// overhead to every build cycle.
const CAUTION_COMMAND_PATTERNS: Array<{ re: RegExp; reason: string }> = [
  { re: /\brm\s+-[a-z]*[rf]/i, reason: "recursive or force-delete" },
  { re: /\bgit\s+push\b.*?(--force|-f)\b/i, reason: "force-push rewrites remote history" },
  { re: /\bgit\s+push\b/i, reason: "pushes to remote" },
  { re: /\bgit\s+reset\s+--hard\b/i, reason: "discards uncommitted changes" },
  { re: /\bgit\s+rebase\b/i, reason: "rewrites commit history" },
  { re: /\bgit\s+clean\s+-[a-z]*f/i, reason: "removes untracked files" },
  { re: /\bgit\s+restore\b/i, reason: "discards working-tree changes" },
  { re: /\bgit\s+checkout\s+(--\s|\.(\s|$))/i, reason: "discards working-tree changes" },
  // Case-sensitive: -D force-deletes, -d only deletes already-merged branches.
  { re: /\bgit\s+branch\s+-[a-z]*D/, reason: "force-deletes a branch" },
  { re: /\btruncate\b/i, reason: "truncates a file" },
  { re: /\bfind\b[^|]*\s-delete\b/i, reason: "deletes matched files" },
  { re: /\bsudo\b/i, reason: "elevated privileges" },
  { re: /\bchmod\b/i, reason: "changes file permissions" },
  { re: /\bapt(-get)?\s+(install|remove|purge)\b/i, reason: "modifies system packages" },
  { re: /\bnpm\s+(install|uninstall)\s+-g\b/i, reason: "modifies global packages" },
  { re: /\bpip\s+install\b/i, reason: "installs Python package" },
  { re: /\byarn\s+global\b/i, reason: "modifies global packages" },
  { re: /\bdocker\s+(rm|rmi|system\s+prune)\b/i, reason: "destroys containers or images" },
  { re: /\bkubectl\s+(delete|drain)\b/i, reason: "removes Kubernetes resources" },
];

export function assessCommand(cmd: string): { level: "dangerous" | "caution"; reason: string } | null {
  for (const p of DANGEROUS_COMMAND_PATTERNS) {
    if (p.re.test(cmd)) return { level: "dangerous", reason: p.reason };
  }
  for (const p of CAUTION_COMMAND_PATTERNS) {
    if (p.re.test(cmd)) return { level: "caution", reason: p.reason };
  }
  return null;
}

export function buildRaw(tool: string, pattern: string | null): string {
  return pattern === null ? tool : `${tool}(${pattern})`;
}

export function parseRaw(raw: string): { tool: string; pattern: string | null } | null {
  const m = raw.match(/^([A-Z][A-Za-z0-9_]*)(?:\((.*)\))?$/s);
  if (!m) return null;
  return { tool: m[1], pattern: m[2] ?? null };
}

export function isHardDenyAllow(raw: string): { hard: boolean; reason?: string } {
  for (const h of HARD_DENY_ALLOW_PATTERNS) {
    if (h.test(raw)) return { hard: true, reason: h.reason };
  }
  return { hard: false };
}

export function classify(
  rule: { scope: Scope; effect: Effect; tool: string; pattern: string | null; raw: string },
): { risk: RiskLevel; reason?: string } {
  if (rule.effect !== "allow") {
    // Deny/ask rules are inherently safe — they restrict, not expand.
    return { risk: "safe" };
  }

  const hd = isHardDenyAllow(rule.raw);
  if (hd.hard) return { risk: "dangerous", reason: hd.reason };

  // Whole tool, no pattern.
  if (rule.pattern === null) {
    if (["Bash", "Write", "Edit", "MultiEdit"].includes(rule.tool)) {
      return { risk: "dangerous", reason: `allows every ${rule.tool} call` };
    }
    if (rule.tool === "WebFetch") return { risk: "moderate", reason: "any URL" };
    return { risk: "safe" };
  }

  if (rule.tool === "Bash") {
    const pat = rule.pattern.trim();
    if (pat === "*" || pat === ":*" || pat === "**") {
      return { risk: "dangerous", reason: "matches any command" };
    }
    if (pat.endsWith(":*")) {
      const prefix = pat.slice(0, -2).trim().split(/\s+/)[0];
      if (rule.scope === "global" && BROAD_GLOBAL_BASH_PREFIXES.has(prefix)) {
        return { risk: "broad", reason: `globally allows all '${prefix}' commands` };
      }
      return { risk: "moderate", reason: `prefix match: '${prefix} ...'` };
    }
    return { risk: "safe" };
  }

  if (PATH_TOOLS.has(rule.tool)) {
    const pat = rule.pattern;
    if (pat === "**" || pat === "*") {
      return { risk: rule.scope === "global" ? "dangerous" : "broad", reason: "every file" };
    }
    if (pat.startsWith("/") || pat.startsWith("../")) {
      return { risk: "dangerous", reason: "path outside cwd" };
    }
    if (pat.endsWith("/**") || pat.endsWith("/*")) {
      const depth = pat.split("/").length;
      if (depth <= 2) return { risk: "broad", reason: "matches top-level directory" };
      return { risk: "moderate", reason: "directory glob" };
    }
    return { risk: "safe" };
  }

  return { risk: "safe" };
}

// Build 1-3 suggestions (exact / prefix / broad) from a live tool-use request.
export function suggestRules(req: ToolUseRequest, scope: Scope): RuleSuggestion[] {
  const out: RuleSuggestion[] = [];
  const push = (effect: Effect, tool: string, pattern: string | null, label: string) => {
    const raw = buildRaw(tool, pattern);
    const { risk, reason } = classify({ scope, effect, tool, pattern, raw });
    const hd: { hard: boolean; reason?: string } =
      effect === "allow" ? isHardDenyAllow(raw) : { hard: false };
    out.push({
      label,
      rule: { scope, effect, tool, pattern, raw },
      risk,
      riskReason: reason ?? hd.reason,
      requiresConfirm: risk === "broad" || risk === "dangerous",
      hardDeny: hd.hard,
    });
  };

  if (req.tool === "Bash") {
    const cmd = typeof req.input.command === "string" ? req.input.command.trim() : "";
    if (!cmd) return [];
    const head = cmd.split(/\s+/)[0];
    push("allow", "Bash", cmd, `Always allow this exact command`);
    if (head && head !== cmd) {
      push("allow", "Bash", `${head}:*`, `Always allow any \`${head} ...\` command`);
    }
    push("allow", "Bash", null, `Always allow ANY Bash command`);
    return out;
  }

  if (PATH_TOOLS.has(req.tool)) {
    const fp = typeof req.input.file_path === "string" ? req.input.file_path : "";
    const rel = toRelative(fp, req.cwd);
    if (rel) {
      push("allow", req.tool, rel, `Always allow ${req.tool} on \`${rel}\``);
      const dir = rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "";
      if (dir) push("allow", req.tool, `${dir}/**`, `Always allow ${req.tool} in \`${dir}/\``);
    }
    push("allow", req.tool, null, `Always allow ANY ${req.tool}`);
    return out;
  }

  // Generic fallback: only whole-tool suggestion.
  push("allow", req.tool, null, `Always allow ${req.tool}`);
  return out;
}

export function toRelative(filePath: string, cwd: string): string | null {
  if (!filePath) return null;
  if (!filePath.startsWith("/")) return filePath;
  if (!cwd) return filePath;
  const c = cwd.endsWith("/") ? cwd : cwd + "/";
  if (filePath === cwd) return ".";
  if (filePath.startsWith(c)) return filePath.slice(c.length);
  return filePath;
}
