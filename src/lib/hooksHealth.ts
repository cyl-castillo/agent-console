// Hooks health: is the bridge between the CLI and the console actually alive?
//
// Every feature that makes this a console rather than a terminal — approval
// modal, proof ledger, snapshots, session resume, the "working" pill — rides
// on hook events. When hooks silently don't run (a directory Claude Code
// doesn't trust, a settings.json another tool rewrote, a bridge binary that
// failed to copy) all of those go dark at once and nothing says so: the
// terminal keeps working, so the failure looks like "the app does less than
// the README promised".
//
// This module is the pure half: it turns two independent signals into a
// verdict. (1) Keystrokes the user submits in a terminal (Enter after typing
// something) — observed by the PTY, no hook involved. (2) Hook events, per
// terminal. A terminal where a live Claude session has swallowed several
// submissions without a single hook event is where hooks are NOT reporting.
// "Live Claude session" comes from `claude agents --json` (pid ancestry, see
// resumeHandles.ts), which is also hook-independent — so the verdict never
// rests on the thing it's judging.

/// Submissions a terminal must swallow before it counts as suspicious. One
/// Enter can be an empty prompt or a slash command the CLI answers locally.
export const STALE_SUBMITS = 2;
/// How long the first unanswered submission must be old. A hook fires within
/// milliseconds of the prompt; this is generous so a slow machine never
/// produces a false alarm.
export const STALE_AFTER_MS = 30_000;

export interface TermSignal {
  /// Enter-after-typing events since the last hook event from this terminal.
  unansweredSubmits: number;
  /// When the first of those happened (0 = none pending).
  firstUnansweredMs: number;
  /// Printable characters typed since the last Enter.
  typedSinceEnter: number;
  /// Last hook event attributed to this terminal (0 = never).
  lastEventMs: number;
}

export const EMPTY_SIGNAL: TermSignal = {
  unansweredSubmits: 0,
  firstUnansweredMs: 0,
  typedSinceEnter: 0,
  lastEventMs: 0,
};

/// Fold one chunk of terminal input into the signal. `\r` (Enter) after at
/// least one printable character is a submission; escape sequences (arrows,
/// mouse reports) are not typing.
export function noteInput(sig: TermSignal, data: string, now: number): TermSignal {
  if (data.length === 0) return sig;
  // Anything starting with ESC is a key/mouse report, never typed text.
  if (data.startsWith("\x1b")) return sig;
  let next = { ...sig };
  for (const ch of data) {
    if (ch === "\r" || ch === "\n") {
      if (next.typedSinceEnter > 0) {
        next = {
          ...next,
          unansweredSubmits: next.unansweredSubmits + 1,
          firstUnansweredMs: next.firstUnansweredMs || now,
          typedSinceEnter: 0,
        };
      }
    } else if (ch.charCodeAt(0) >= 32) {
      next.typedSinceEnter += 1;
    }
  }
  return next;
}

/// A hook event arrived from this terminal: everything pending is answered.
export function noteEvent(sig: TermSignal, now: number): TermSignal {
  return { ...sig, unansweredSubmits: 0, firstUnansweredMs: 0, lastEventMs: now };
}

/// Terminals whose pending submissions are many enough and old enough.
export function suspiciousTerms(perTerm: Record<string, TermSignal>, now: number): string[] {
  return Object.entries(perTerm)
    .filter(
      ([, s]) =>
        s.unansweredSubmits >= STALE_SUBMITS &&
        s.firstUnansweredMs > 0 &&
        now - s.firstUnansweredMs >= STALE_AFTER_MS,
    )
    .map(([id]) => id);
}

export type HooksVerdict =
  /// Hooks are not installed: none of the hook-driven features can work.
  | { kind: "off" }
  /// Installed, but no event has ever arrived — nothing to judge yet.
  | { kind: "silent" }
  /// Events are flowing.
  | { kind: "ok"; lastEventMs: number }
  /// Live Claude sessions swallowed submissions without any hook event.
  | { kind: "stale"; termIds: string[]; lastEventMs: number | null };

export interface AssessInput {
  installed: boolean;
  lastEventMs: number | null;
  perTerm: Record<string, TermSignal>;
  /// Terminals with a live Claude session, proven by `claude agents --json`.
  liveClaudeTerms: ReadonlySet<string>;
  now: number;
}

export function assessHooksHealth(input: AssessInput): HooksVerdict {
  if (!input.installed) return { kind: "off" };
  const stale = suspiciousTerms(input.perTerm, input.now).filter((t) =>
    input.liveClaudeTerms.has(t),
  );
  if (stale.length > 0) return { kind: "stale", termIds: stale, lastEventMs: input.lastEventMs };
  if (input.lastEventMs === null) return { kind: "silent" };
  return { kind: "ok", lastEventMs: input.lastEventMs };
}

/// "12s" / "3m" / "2h" for the pill.
export function formatAge(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  return `${Math.round(m / 60)}h`;
}
