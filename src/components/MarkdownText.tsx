import { useMemo } from "react";
import { marked } from "marked";

marked.setOptions({
  gfm: true,
  breaks: true,
});

interface Props {
  content: string;
}

/// Renders an assistant text block as markdown. Tolerates partial input —
/// `marked` happily parses unclosed fences and lists during streaming.
export function MarkdownText({ content }: Props) {
  const html = useMemo(() => marked.parse(content, { async: false }) as string, [content]);
  return <div className="markdown" dangerouslySetInnerHTML={{ __html: html }} />;
}
