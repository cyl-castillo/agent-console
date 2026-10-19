/// The composer's echo: what was sent to the terminal and not yet witnessed
/// as a prompt in the ledger. Pure helpers, so the rules have unit tests.

export interface PendingPrompt {
  text: string;
  /// When it was typed into the PTY (ms since epoch).
  ts: number;
}

/// After this long without a witnessed prompt the echo keeps its note, and
/// stops holding the queue back (the hooks may be off — then nothing would
/// ever witness it, and a queued prompt must still go out).
export const UNWITNESSED_GRACE_MS = 5_000;

/// A prompt witnessed by the ledger counts for an echo only when it was
/// recorded at or after the send (minus this slack, for clocks and the hook's
/// own delay). Matching by text alone let an older turn with the same prompt
/// ("ok", "continue") claim a fresh echo at once — and release the queue into
/// a terminal that was still busy.
export const WITNESS_SLACK_MS = 2_000;

/// The echoes a witnessed turn list does not account for yet.
export function unwitnessed<T extends PendingPrompt>(
  pending: T[],
  turns: { prompt: string; ts: number }[],
): T[] {
  return pending.filter(
    (p) => !turns.some((t) => t.prompt.trim() === p.text && t.ts >= p.ts - WITNESS_SLACK_MS),
  );
}

/// True while a recent send is still waiting to be witnessed: the queue
/// holds, so the next prompt lands as its own turn and not into a terminal
/// that is about to get busy.
export function holdsQueue(pending: PendingPrompt[], now: number): boolean {
  return pending.some((p) => now - p.ts < UNWITNESSED_GRACE_MS);
}
