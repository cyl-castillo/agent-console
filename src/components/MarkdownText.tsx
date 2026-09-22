import { useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({
  gfm: true,
  breaks: true,
});

interface Props {
  content: string;
}

/// Renders an assistant text block as markdown. Tolerates partial input —
/// `marked` happily parses unclosed fences and lists during streaming.
///
/// The output is sanitized before it touches the DOM: what lands here is
/// agent output (rooms), SKILL.md bodies pulled from marketplaces and
/// CLAUDE.md/memory files — none of it is ours, and `marked` deliberately
/// passes raw HTML through. Without this, an `<img onerror>` in a room reply
/// would run inside the webview with `invoke` in reach (read files, type into
/// the shell). Belt: DOMPurify here; braces: the CSP in tauri.conf.json.
export function MarkdownText({ content }: Props) {
  const html = useMemo(
    () =>
      DOMPurify.sanitize(marked.parse(content, { async: false }) as string, {
        USE_PROFILES: { html: true },
        // Markdown never needs these; each is a script-execution or
        // navigation vector on its own.
        FORBID_TAGS: ["style", "form", "input", "button", "iframe", "object", "embed"],
        FORBID_ATTR: ["style", "formaction"],
      }),
    [content],
  );
  return <div className="markdown" dangerouslySetInnerHTML={{ __html: html }} />;
}
