import { useState, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, Copy } from "lucide-react";
import { cn } from "@/lib/cn";
import { tokenize, type TokenKind } from "../lib/highlight";
import { ALLOWED_ELEMENTS, linkKind } from "../lib/markdownPolicy";

const TOKEN_CLASS: Record<TokenKind, string | undefined> = {
  comment: "text-faint-foreground italic",
  string: "text-[var(--ok-ink)]",
  number: "text-[var(--warn-ink)]",
  keyword: "text-[var(--accent-ink)]",
  plain: undefined,
};

/// A fenced block: tokens as spans, the language as a tag, a copy button.
function CodeFence({
  code,
  lang,
  onCopy,
}: {
  code: string;
  lang: string;
  onCopy?: (text: string) => void;
}) {
  const [copied, setCopied] = useState(false);
  const text = code.replace(/\n$/, "");
  return (
    <div
      className="group relative rounded-2 border border-border bg-background"
      data-testid="code-fence"
      data-lang={lang || undefined}
    >
      <div className="flex h-6 items-center justify-between border-b border-border px-2 font-sans text-caption text-faint-foreground">
        <span>{lang || "code"}</span>
        {onCopy && (
          <button
            type="button"
            className="unbutton inline-flex items-center gap-1 text-caption text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
            onClick={() => {
              onCopy(text);
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1200);
            }}
            aria-label="Copy code"
          >
            {copied ? <Check size={12} aria-hidden /> : <Copy size={12} aria-hidden />}
            {copied ? "copied" : "copy"}
          </button>
        )}
      </div>
      <pre className="m-0 overflow-x-auto p-2 font-code text-small leading-snug">
        <code>
          {tokenize(text, lang).map((t, i) =>
            t.kind === "plain" ? (
              t.text
            ) : (
              <span key={i} className={TOKEN_CLASS[t.kind]}>
                {t.text}
              </span>
            ),
          )}
        </code>
      </pre>
    </div>
  );
}

function childText(children: ReactNode): string {
  if (typeof children === "string") return children;
  if (Array.isArray(children)) return children.map(childText).join("");
  if (children && typeof children === "object" && "props" in children) {
    return childText((children as { props: { children?: ReactNode } }).props.children);
  }
  return "";
}

/// Model prose, rendered from a markdown tree: GitHub-flavoured (tables,
/// strikethrough, task lists), never from HTML — a literal tag in the reply
/// stays the text it is (no rehype-raw, so raw nodes become text). Web links open outside; a
/// relative path opens the file in Changes when `onOpenFile` is given.
export function MarkdownText({
  text,
  className,
  onOpenFile,
  onCopy,
}: {
  text: string;
  className?: string;
  onOpenFile?: (path: string) => void;
  onCopy?: (text: string) => void;
}) {
  const components: Components = {
    a: ({ href, children }) => {
      const kind = href ? linkKind(href) : "none";
      if (kind === "web") {
        return (
          <a
            href={href}
            target="_blank"
            rel="noreferrer noopener"
            className="text-[var(--accent-ink)] underline decoration-dotted underline-offset-2 hover:decoration-solid"
          >
            {children}
          </a>
        );
      }
      if (kind === "path" && onOpenFile && href) {
        return (
          <button
            type="button"
            className="unbutton inline font-code text-[0.92em] text-[var(--accent-ink)] underline decoration-dotted underline-offset-2 hover:decoration-solid focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
            onClick={() => onOpenFile(href)}
            data-testid="md-path"
          >
            {children}
          </button>
        );
      }
      return <span>{children}</span>;
    },
    pre: ({ children }) => {
      const child = Array.isArray(children) ? children[0] : children;
      const props =
        child && typeof child === "object" && "props" in child
          ? (child as { props: { className?: string; children?: ReactNode } }).props
          : { className: undefined, children };
      const lang = /language-([\w+-]+)/.exec(props.className ?? "")?.[1] ?? "";
      return <CodeFence code={childText(props.children)} lang={lang} onCopy={onCopy} />;
    },
    code: ({ children }) => (
      <code className="rounded-1 bg-surface-2 px-1 font-code text-[0.92em] text-foreground">
        {children}
      </code>
    ),
    p: ({ children }) => <p className="m-0 whitespace-pre-wrap break-words">{children}</p>,
    ul: ({ children, className: c }) => (
      <ul
        className={cn(
          "m-0 flex flex-col gap-0.5 pl-5",
          c === "contains-task-list" ? "list-none pl-1" : "list-disc",
        )}
      >
        {children}
      </ul>
    ),
    ol: ({ children }) => (
      <ol className="m-0 flex flex-col gap-0.5 list-decimal pl-5">{children}</ol>
    ),
    li: ({ children }) => <li className="[&>ul]:mt-0.5 [&>ol]:mt-0.5">{children}</li>,
    input: ({ checked }) => (
      <input
        type="checkbox"
        checked={!!checked}
        readOnly
        disabled
        className="mr-1.5 align-middle"
      />
    ),
    h1: ({ children }) => (
      <div role="heading" aria-level={3} className="font-semibold text-title">
        {children}
      </div>
    ),
    h2: ({ children }) => (
      <div role="heading" aria-level={4} className="font-semibold text-lead">
        {children}
      </div>
    ),
    h3: ({ children }) => (
      <div role="heading" aria-level={5} className="font-semibold text-body">
        {children}
      </div>
    ),
    h4: ({ children }) => (
      <div role="heading" aria-level={6} className="font-semibold text-body">
        {children}
      </div>
    ),
    h5: ({ children }) => (
      <div role="heading" aria-level={6} className="font-semibold text-body">
        {children}
      </div>
    ),
    h6: ({ children }) => (
      <div role="heading" aria-level={6} className="font-semibold text-body">
        {children}
      </div>
    ),
    blockquote: ({ children }) => (
      <blockquote className="m-0 border-l-2 border-border pl-3 text-muted-foreground">
        {children}
      </blockquote>
    ),
    hr: () => <hr className="m-0 border-0 border-t border-border" />,
    table: ({ children }) => (
      <div className="overflow-x-auto" data-testid="md-table">
        <table className="w-max min-w-full border-collapse text-small">{children}</table>
      </div>
    ),
    th: ({ children }) => (
      <th className="border border-border bg-surface-2 px-2 py-1 text-left font-semibold">
        {children}
      </th>
    ),
    td: ({ children }) => <td className="border border-border px-2 py-1 align-top">{children}</td>,
  };
  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 font-sans text-body leading-normal text-foreground",
        className,
      )}
    >
      <Markdown
        remarkPlugins={[remarkGfm]}
        allowedElements={ALLOWED_ELEMENTS}
        unwrapDisallowed
        urlTransform={(url) => (linkKind(url) === "none" ? "" : url)}
        components={components}
      >
        {text}
      </Markdown>
    </div>
  );
}
