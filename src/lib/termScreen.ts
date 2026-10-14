/// What a session's terminal is showing right now. Each mounted Terminal
/// registers a reader over its xterm buffer; voice commands read the last
/// lines to know whether the agent is asking something before sending keys.

type Reader = (lines: number) => string[];

const readers = new Map<string, Reader>();

export function registerScreenReader(sessionId: string, read: Reader): void {
  readers.set(sessionId, read);
}

export function unregisterScreenReader(sessionId: string): void {
  readers.delete(sessionId);
}

/// The last `lines` lines of the session's screen, oldest first. Empty when
/// the session has no mounted terminal.
export function readScreen(sessionId: string, lines = 15): string[] {
  try {
    return readers.get(sessionId)?.(lines) ?? [];
  } catch {
    return [];
  }
}
