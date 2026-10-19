/// What the agent's prose may become. Raw HTML is never parsed (no rehype-raw):
/// a literal `<img onerror>` in a reply stays the text it is. Elements
/// outside this list are unwrapped to their text.
export const ALLOWED_ELEMENTS = [
  "p",
  "br",
  "strong",
  "em",
  "del",
  "code",
  "pre",
  "a",
  "ul",
  "ol",
  "li",
  "input",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "blockquote",
  "hr",
  "table",
  "thead",
  "tbody",
  "tr",
  "th",
  "td",
];

const HTTP = /^https?:\/\//i;
const SCHEME = /^[a-z][a-z0-9+.-]*:/i;
/// A link the agent may point at: the web, or a path inside the project.
export function linkKind(href: string): "web" | "path" | "none" {
  if (HTTP.test(href)) return "web";
  if (SCHEME.test(href) || href.startsWith("//") || href.startsWith("#")) return "none";
  if (/^[\w./@-][^\s]*$/.test(href)) return "path";
  return "none";
}
