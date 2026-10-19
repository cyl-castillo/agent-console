import type { TranscriptItem } from "../../../stores/transcriptStore";

/// What the transcript says about one turn: the paragraphs the agent wrote,
/// how many tools it called, and the call still running (no result yet).
export interface TurnStream {
  texts: { id: string; text: string }[];
  toolUses: number;
  running: { name: string; input: string } | null;
}

export const EMPTY_STREAM: TurnStream = { texts: [], toolUses: 0, running: null };

/// The ledger's clock (the hook's arrival) and the CLI's (the transcript line)
/// are the same machine's, but a prompt is witnessed a beat before the CLI
/// writes it; a live turn's items may also land a beat before the hook.
export const CLOCK_SLACK_MS = 1500;

/// Items of `items` that belong to the turn opened at `ts` and closed at
/// `endTs` (null while live): inside the window, after the prompt that
/// starts it, before the next one.
export function streamFor(
  items: readonly TranscriptItem[],
  turn: { ts: number; endTs: number | null },
): TurnStream {
  const from = turn.ts - CLOCK_SLACK_MS;
  const to = turn.endTs === null ? Number.POSITIVE_INFINITY : turn.endTs + CLOCK_SLACK_MS;
  const inWindow = items.filter((i) => i.ts >= from && i.ts <= to);
  if (inWindow.length === 0) return EMPTY_STREAM;
  const firstPrompt = inWindow.findIndex((i) => i.kind === "user");
  let mine = firstPrompt >= 0 ? inWindow.slice(firstPrompt + 1) : inWindow;
  const nextPrompt = mine.findIndex((i) => i.kind === "user");
  if (nextPrompt >= 0) mine = mine.slice(0, nextPrompt);
  const texts: TurnStream["texts"] = [];
  const open = new Map<string, { name: string; input: string }>();
  let toolUses = 0;
  for (const i of mine) {
    if (i.kind === "text") texts.push({ id: i.id, text: i.text });
    else if (i.kind === "tool_use") {
      toolUses += 1;
      open.set(i.id, { name: i.name, input: i.input });
    } else if (i.kind === "tool_result") open.delete(i.id);
  }
  const running = turn.endTs === null ? ([...open.values()].pop() ?? null) : null;
  if (texts.length === 0 && toolUses === 0 && !running) return EMPTY_STREAM;
  return { texts, toolUses, running };
}

/// What the turn's "Agent" block shows: the streamed paragraphs when the
/// transcript has them, else the ledger's closing summary.
export function saidText(stream: TurnStream, summary: string): string {
  return stream.texts.length > 0 ? stream.texts.map((t) => t.text).join("\n\n") : summary;
}
