/// What the console says on its own: a turn that ended, a turn that failed.
/// Short on purpose — a spoken line is heard once, not scanned.

/// The first `max` sentences of `text` (sentence = ends in . ! ? followed by
/// a space or the end). Text without a sentence end comes back whole.
export function firstSentences(text: string, max: number): string {
  let out = "";
  let count = 0;
  const chars = Array.from(text);
  for (let i = 0; i < chars.length && count < max; i++) {
    const c = chars[i];
    out += c;
    const ends =
      (c === "." || c === "!" || c === "?") && (i + 1 === chars.length || /\s/.test(chars[i + 1]));
    if (ends) count += 1;
  }
  return out.trim();
}

/// Spoken line for an ended turn. The summary is the agent's last message;
/// the backend strips markdown before speaking, this only shortens it. The
/// session name leads when the turn isn't the one on screen.
export function turnAnnouncement(
  summary: string | undefined,
  sessionName: string | undefined,
  isActive: boolean,
  es: boolean,
): string {
  const who = !isActive && sessionName ? `${sessionName}: ` : "";
  const body = summary?.trim()
    ? firstSentences(summary.trim(), 2)
    : es
      ? "Turno terminado."
      : "Turn finished.";
  return who + body;
}
