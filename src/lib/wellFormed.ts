/// Strings that cross the IPC boundary are parsed by serde_json, which
/// rejects a lone UTF-16 surrogate ("lone leading surrogate in hex escape").
/// A JS string cut by code unit can start or end in the middle of a pair —
/// one emoji at the wrong offset and the whole payload is refused. These
/// helpers keep every cut on a code point boundary.

const ANY_SURROGATE = /[\uD800-\uDFFF]/;
const PAIR_OR_LONE = /[\uD800-\uDBFF][\uDC00-\uDFFF]|[\uD800-\uDFFF]/g;

/// Replace every lone surrogate with U+FFFD; valid pairs are left alone.
/// (String.prototype.toWellFormed does the same, but is missing from older
/// WebKitGTK builds the app still runs on.)
export function toWellFormed(s: string): string {
  if (!ANY_SURROGATE.test(s)) return s;
  return s.replace(PAIR_OR_LONE, (m) => (m.length === 2 ? m : "�"));
}

/// The last `max` UTF-16 code units of `s`, never starting inside a pair,
/// and well formed throughout. May be one unit shorter than `max`.
export function tailWellFormed(s: string, max: number): string {
  if (s.length <= max) return toWellFormed(s);
  let start = s.length - max;
  const first = s.charCodeAt(start);
  // The cut landed on a low surrogate: its high half was left behind.
  if (first >= 0xdc00 && first <= 0xdfff) start += 1;
  return toWellFormed(s.slice(start));
}
