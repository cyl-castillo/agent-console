/// The model's prose, as data: a tiny Markdown subset parsed into blocks the
/// UI renders as React elements — never as HTML. Headings, bullets, numbered
/// lists, fenced code, inline code and emphasis are enough for summaries;
/// anything else (raw HTML, links with javascript:, images) stays literal
/// text. This is the "0 HTML from the model" gate of UI-2026-10.md.
export type Inline =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong"; text: string };

export type Block =
  | { kind: "paragraph"; inlines: Inline[] }
  | { kind: "heading"; level: 1 | 2 | 3; inlines: Inline[] }
  | { kind: "list"; ordered: boolean; items: Inline[][] }
  | { kind: "code"; lang: string | null; text: string };

export function parseInlines(line: string): Inline[] {
  const out: Inline[] = [];
  // `code` and **strong**, left to right; everything else is text.
  const re = /`([^`]+)`|\*\*([^*]+)\*\*/g;
  let last = 0;
  for (const m of line.matchAll(re)) {
    const i = m.index ?? 0;
    if (i > last) out.push({ kind: "text", text: line.slice(last, i) });
    if (m[1] !== undefined) out.push({ kind: "code", text: m[1] });
    else out.push({ kind: "strong", text: m[2] });
    last = i + m[0].length;
  }
  if (last < line.length) out.push({ kind: "text", text: line.slice(last) });
  return out;
}

export function parseMarkdownLite(src: string): Block[] {
  const blocks: Block[] = [];
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  let para: string[] = [];
  const flush = () => {
    if (para.length) {
      blocks.push({ kind: "paragraph", inlines: parseInlines(para.join(" ")) });
      para = [];
    }
  };
  while (i < lines.length) {
    const line = lines[i];
    const fence = /^```(\w*)\s*$/.exec(line);
    if (fence) {
      flush();
      const lang = fence[1] || null;
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) body.push(lines[i++]);
      i++; // closing fence (or EOF)
      blocks.push({ kind: "code", lang, text: body.join("\n") });
      continue;
    }
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      blocks.push({
        kind: "heading",
        level: heading[1].length as 1 | 2 | 3,
        inlines: parseInlines(heading[2]),
      });
      i++;
      continue;
    }
    const bullet = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(line);
    if (bullet) {
      flush();
      const ordered = /^\s*\d/.test(line);
      const items: Inline[][] = [];
      while (i < lines.length) {
        const m = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(lines[i]);
        if (!m) break;
        items.push(parseInlines(m[1]));
        i++;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    if (line.trim() === "") {
      flush();
      i++;
      continue;
    }
    para.push(line.trim());
    i++;
  }
  flush();
  return blocks;
}
