import type { ApprovalRequest } from "../types/domain";

/// D14: the session's network perimeter, enforced where the console already
/// decides — the approval gate. Mirrors identity_service::host_allowed.
export function hostAllowed(host: string, allowed: string[]): boolean {
  const h = host.trim().toLowerCase();
  return allowed.some((raw) => {
    const a = raw.trim().toLowerCase();
    if (a.startsWith(".")) {
      const suffix = a.slice(1);
      return h === suffix || h.endsWith(`.${suffix}`);
    }
    return h === a;
  });
}

const URL_RE = /\bhttps?:\/\/([a-z0-9.-]+)(?::\d+)?/gi;
const SSH_RE =
  /\b(?:ssh|scp|sftp|rsync)\b[^|;&]*?\b(?:[\w.-]+@)?([a-z0-9][a-z0-9.-]+\.[a-z]{2,})/gi;

/// Every host a request would reach: URLs in a Bash command, the url of
/// WebFetch, ssh/scp/rsync targets. Empty for requests that stay local.
export function hostsIn(req: ApprovalRequest): string[] {
  const inp = req.input ?? {};
  const out = new Set<string>();
  const scan = (text: string) => {
    for (const m of text.matchAll(URL_RE)) out.add(m[1].toLowerCase());
    for (const m of text.matchAll(SSH_RE)) out.add(m[1].toLowerCase());
  };
  if (req.tool === "Bash" && typeof inp.command === "string") scan(inp.command);
  if (req.tool === "WebFetch" && typeof inp.url === "string") scan(inp.url);
  if (req.tool.startsWith("mcp__")) {
    for (const v of Object.values(inp)) if (typeof v === "string") scan(v);
  }
  return [...out];
}

/// Hosts the request reaches that fall outside a declared perimeter. An
/// undeclared perimeter (empty list) means no judgement: [].
export function hostsOutside(req: ApprovalRequest, allowed: string[] | undefined): string[] {
  if (!allowed || allowed.length === 0) return [];
  return hostsIn(req).filter((h) => !hostAllowed(h, allowed));
}

/// `urn:agent-console:agent:<uuid>` — an agent's own id, never the human's.
export function mintAgentId(): string {
  return `urn:agent-console:agent:${crypto.randomUUID()}`;
}

export function parseHosts(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[\s,]+/)
        .map((s) => s.trim().toLowerCase())
        .filter(Boolean),
    ),
  ];
}
