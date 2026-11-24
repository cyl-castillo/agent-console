/// Files dropped on the conversation: their paths go into the draft, quoted,
/// the form every agent CLI reads as "this file". Pure, so it has unit tests.

/// One path as the agent should read it. A path holding a double quote is
/// wrapped in single quotes instead; nothing here is run by a shell — the
/// draft is prose the user still reviews and sends.
export function quotePath(path: string): string {
  return path.includes('"') ? `'${path}'` : `"${path}"`;
}

/// The draft after dropping `paths` on it: appended, one space apart, with a
/// trailing space so typing continues naturally. Nothing dropped, same draft.
export function draftWithDropped(draft: string, paths: string[]): string {
  const text = paths
    .filter((p) => p.length > 0)
    .map(quotePath)
    .join(" ");
  if (!text) return draft;
  const sep = draft.length === 0 || /\s$/.test(draft) ? "" : " ";
  return `${draft}${sep}${text} `;
}
