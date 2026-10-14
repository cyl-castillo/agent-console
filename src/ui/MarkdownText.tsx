import { parseMarkdownLite, type Inline } from "../lib/markdownLite";
import { cn } from "@/lib/cn";

function Inlines({ parts }: { parts: Inline[] }) {
  return (
    <>
      {parts.map((p, i) =>
        p.kind === "code" ? (
          <code
            key={i}
            className="rounded-1 bg-surface-2 px-1 font-code text-[0.92em] text-foreground"
          >
            {p.text}
          </code>
        ) : p.kind === "strong" ? (
          <strong key={i} className="font-semibold">
            {p.text}
          </strong>
        ) : (
          <span key={i}>{p.text}</span>
        ),
      )}
    </>
  );
}

/// Model prose rendered from parsed blocks, never from HTML: the text the
/// agent produced goes through React's escaping on every path.
export function MarkdownText({ text, className }: { text: string; className?: string }) {
  const blocks = parseMarkdownLite(text);
  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 font-sans text-body leading-normal text-foreground",
        className,
      )}
    >
      {blocks.map((b, i) => {
        switch (b.kind) {
          case "heading": {
            const size = b.level === 1 ? "text-title" : b.level === 2 ? "text-lead" : "text-body";
            return (
              <div
                key={i}
                className={cn("font-semibold", size)}
                role="heading"
                aria-level={b.level + 2}
              >
                <Inlines parts={b.inlines} />
              </div>
            );
          }
          case "list":
            return b.ordered ? (
              <ol key={i} className="m-0 list-decimal pl-5">
                {b.items.map((it, j) => (
                  <li key={j}>
                    <Inlines parts={it} />
                  </li>
                ))}
              </ol>
            ) : (
              <ul key={i} className="m-0 list-disc pl-5">
                {b.items.map((it, j) => (
                  <li key={j}>
                    <Inlines parts={it} />
                  </li>
                ))}
              </ul>
            );
          case "code":
            return (
              <pre
                key={i}
                className="m-0 overflow-auto rounded-2 border border-border bg-background p-2 font-code text-small leading-snug"
              >
                {b.text}
              </pre>
            );
          default:
            return (
              <p key={i} className="m-0 whitespace-pre-wrap break-words">
                <Inlines parts={b.inlines} />
              </p>
            );
        }
      })}
    </div>
  );
}
