import type { TimelineTurn } from "../stores/proofStore";
import type { OpencodeModel } from "../types/domain";

/// F10: when a Claude or Codex session hits its budget or the plan's quota,
/// the work can continue on an open-weight model through OpenCode. This is
/// the hand-over: what the next agent is told, and which model to suggest.
/// Pure, so the vectors are tests; nothing here sends anything.

export type HandoffReason = "budget" | "quota";

/// Providers that serve open-weight models (local or hosted). The catalog
/// is whatever the user configured; this only orders the suggestion.
const OPEN_WEIGHT_PROVIDERS = [
  "ollama",
  "lmstudio",
  "llama.cpp",
  "groq",
  "together",
  "fireworks",
  "deepinfra",
  "cerebras",
  "openrouter",
  "huggingface",
];
const OPEN_WEIGHT_NAMES =
  /llama|qwen|deepseek|mistral|mixtral|gemma|gpt-oss|phi|glm|kimi|devstral|codestral/i;

/// The model to offer first: a local open-weight one, then any open-weight
/// one, then the first of the catalog. `null` when the catalog is empty —
/// the card then says OpenCode has no models configured.
export function suggestOpencodeModel(models: OpencodeModel[]): OpencodeModel | null {
  if (models.length === 0) return null;
  const score = (m: OpencodeModel) => {
    const provider = m.provider.toLowerCase();
    const local = provider === "ollama" || provider === "lmstudio" || provider === "llama.cpp";
    const openProvider = OPEN_WEIGHT_PROVIDERS.some((p) => provider.includes(p));
    const openName = OPEN_WEIGHT_NAMES.test(`${m.id} ${m.name}`);
    return (local ? 4 : 0) + (openProvider ? 2 : 0) + (openName ? 1 : 0);
  };
  return [...models].sort((a, b) => score(b) - score(a))[0] ?? null;
}

export interface HandoffContext {
  /// The session's last turn as the ledger holds it (may be undefined when
  /// the ledger saw nothing for it yet).
  turn?: TimelineTurn;
  fromEngine: string;
  fromModel?: string;
  reason: HandoffReason;
}

/// What the ledger line carries about the hand-over (bounded).
export function handoffPayload(ctx: HandoffContext): Record<string, unknown> {
  const t = ctx.turn;
  return {
    prompt: t ? clip(t.prompt, 500) : null,
    files: t ? t.files.slice(0, 50).map((f) => f.path) : [],
    verification: t?.verification?.status ?? null,
    summary: t ? clip(t.summary, 500) : null,
  };
}

function clip(s: string, n: number): string {
  const t = s.trim();
  return t.length > n ? `${t.slice(0, n)}…` : t;
}

/// The seed typed into the new OpenCode session — not sent: the human reads
/// it first. It names why the previous agent stopped, what it was asked,
/// what it touched and what the console's last verdict was, then asks for
/// the same evidence every turn owes.
export function handoffPrompt(ctx: HandoffContext): string {
  const why =
    ctx.reason === "budget" ? "its cost cap was reached" : "the plan's usage quota ran out";
  const from = `${ctx.fromEngine}${ctx.fromModel ? ` (${ctx.fromModel})` : ""}`;
  const t = ctx.turn;
  const lines: string[] = [
    `You are taking over from ${from}, which stopped because ${why}. Continue the work; do not start over.`,
  ];
  if (t?.prompt) lines.push(`\nThe task it was given:\n${clip(t.prompt, 1500)}`);
  if (t && t.files.length > 0) {
    const shown = t.files.slice(0, 30).map((f) => `- ${f.status} ${f.path}`);
    const more = t.files.length > 30 ? `\n- … ${t.files.length - 30} more` : "";
    lines.push(`\nFiles it touched so far:\n${shown.join("\n")}${more}`);
  }
  if (t?.summary) lines.push(`\nIts last words:\n${clip(t.summary, 800)}`);
  if (t?.verification) {
    lines.push(
      `\nThe console's verdict on that turn: ${t.verification.status} — ${t.verification.reason}`,
    );
  }
  lines.push(
    "\nFirst read the files above and the current diff, then continue. When you claim something passes, run the project's test gate and report the exit code.",
  );
  return lines.join("\n");
}

/// Sessions whose last witnessed turn ended on the plan's quota, with no
/// hand-over recorded after it: the ones the Inbox asks about.
export function quotaOuts(
  turns: TimelineTurn[],
  events: { kind: string; termId?: string; ts: number; payload: Record<string, unknown> }[],
): TimelineTurn[] {
  const last = new Map<string, TimelineTurn>();
  for (const t of turns) if (t.termId) last.set(t.termId, t);
  const out: TimelineTurn[] = [];
  for (const t of last.values()) {
    if (!t.failed || t.error !== "rate_limit") continue;
    const handed = events.some(
      (e) =>
        e.kind === "model_switch" &&
        e.termId === t.termId &&
        e.ts >= t.ts &&
        (e.payload.reason === "quota" || e.payload.reason === "budget"),
    );
    if (!handed) out.push(t);
  }
  return out;
}
