import { create } from "zustand";

/// F3 "nothing happens out of sight": what the terminal is showing that the
/// chat must surface — the CLI still booting, a folder-trust question, a
/// login the CLI wants, a command the shell could not find. Detected by
/// text patterns on the PTY output per session; these only raise cards and
/// open the drawer, never act on the user's behalf (CHAT-2026-10 §7).
export interface TermSignals {
  /// Launch typed, no CLI prompt seen yet.
  booting: boolean;
  bootedAt?: number;
  /// The CLI's own prompt/banner appeared.
  ready?: number;
  trust?: number;
  login?: number;
  notFound?: number;
}

const EMPTY: TermSignals = { booting: false };
/// Rolling tail of plain text kept per session for the matchers.
const TAIL_CHARS = 4000;
/// Booting cannot last forever: after this the card stops claiming it.
export const BOOT_TIMEOUT_MS = 60_000;

// eslint-disable-next-line no-control-regex
const ANSI = /\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\x1b[()][AB012]|\x1b[=>78]|\r/g;
export const PATTERNS = {
  trust:
    /do you trust the files in this folder|is this a project you created or one you trust|trust this folder/i,
  login:
    /please run \/login|run `?\/login`?|not logged in|oauth token (has )?expired|invalid api key|please log ?in|authentication (required|failed)/i,
  notFound:
    /command not found|is not recognized as an internal or external command|no such file or directory: (claude|codex)|not found: (claude|codex)/i,
  // The CLI's own banner, never a bare prompt glyph: shells like starship
  // draw ❯ too, and that would call a shell "ready" before the launch.
  ready:
    /welcome (back|to claude code|to codex)|tips for getting started|what would you like to do|openai codex/i,
};

export function stripAnsi(s: string): string {
  return s.replace(ANSI, "");
}

/// Apply one output chunk to a session's signals. Pure, for the tests.
export function detect(prev: TermSignals, tail: string, chunk: string, now: number) {
  const text = (tail + stripAnsi(chunk)).slice(-TAIL_CHARS);
  const plain = stripAnsi(chunk);
  const next: TermSignals = { ...prev };
  if (PATTERNS.trust.test(plain)) next.trust = now;
  if (PATTERNS.login.test(plain)) next.login = now;
  if (PATTERNS.notFound.test(plain)) next.notFound = now;
  if (PATTERNS.ready.test(plain)) {
    next.ready = now;
    next.booting = false;
    // The CLI is up: a trust/login question it just answered is behind us
    // only when it came before this banner — keep ones raised in this chunk.
    if (!PATTERNS.trust.test(plain)) delete next.trust;
    if (!PATTERNS.login.test(plain)) delete next.login;
  }
  return { next, tail: text };
}

interface TermSignalsState {
  bySession: Record<string, TermSignals>;
  tails: Record<string, string>;
  note: (sessionId: string, chunk: string) => void;
  markBooting: (sessionId: string) => void;
  /// A witnessed prompt or any ledger line: the CLI is live, questions are over.
  clear: (sessionId: string, keys?: (keyof TermSignals)[]) => void;
  forget: (sessionId: string) => void;
}

export const useTermSignalsStore = create<TermSignalsState>((set, get) => ({
  bySession: {},
  tails: {},
  note: (sessionId, chunk) => {
    const prev = get().bySession[sessionId] ?? EMPTY;
    const { next, tail } = detect(prev, get().tails[sessionId] ?? "", chunk, Date.now());
    const changed =
      next.booting !== prev.booting ||
      next.ready !== prev.ready ||
      next.trust !== prev.trust ||
      next.login !== prev.login ||
      next.notFound !== prev.notFound;
    // Tails change on every chunk; signals rarely — only the latter re-renders.
    get().tails[sessionId] = tail;
    if (changed) set((s) => ({ bySession: { ...s.bySession, [sessionId]: next } }));
  },
  // Booting starts at the launch; a banner already seen (a fixture, a
  // replay) means the CLI is up and the claim is void from the start.
  markBooting: (sessionId) =>
    set((s) => {
      const prev = s.bySession[sessionId];
      return {
        bySession: {
          ...s.bySession,
          [sessionId]: { booting: true, bootedAt: Date.now(), ready: prev?.ready },
        },
      };
    }),
  clear: (sessionId, keys) =>
    set((s) => {
      const cur = s.bySession[sessionId];
      if (!cur) return s;
      const next: TermSignals = { ...cur, booting: false };
      for (const k of keys ?? ["trust", "login", "notFound"]) delete next[k];
      return { bySession: { ...s.bySession, [sessionId]: next } };
    }),
  forget: (sessionId) =>
    set((s) => {
      const bySession = { ...s.bySession };
      delete bySession[sessionId];
      delete get().tails[sessionId];
      return { bySession };
    }),
}));

/// The signals of one session (a stable empty when none).
export function signalsFor(bySession: Record<string, TermSignals>, id: string | undefined) {
  return id ? (bySession[id] ?? EMPTY) : EMPTY;
}

/// Booting, as the card sees it: flagged, no prompt seen, not timed out.
export function isBooting(sig: TermSignals, now: number): boolean {
  return sig.booting && !sig.ready && now - (sig.bootedAt ?? now) < BOOT_TIMEOUT_MS;
}
