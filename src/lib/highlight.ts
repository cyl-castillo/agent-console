/// A small, dependency-free code tokenizer for fenced blocks: comments,
/// strings, numbers and a shared keyword set, enough to read a snippet at a
/// glance. Output is tokens the renderer turns into spans — never HTML.
export type TokenKind = "comment" | "string" | "number" | "keyword" | "plain";
export interface Token {
  kind: TokenKind;
  text: string;
}

const KEYWORDS = new Set(
  (
    "const let var function return if else for while do switch case default import export from " +
    "class extends new async await try catch finally throw typeof instanceof in of type interface " +
    "enum implements static readonly this super null undefined true false void yield " +
    "fn pub use mut impl struct trait where loop match mod crate self Self ref move dyn unsafe " +
    "def lambda None True False not and or elif with as pass raise global nonlocal is " +
    "package go chan defer select range map func " +
    "echo then fi esac done local exit set"
  ).split(" "),
);

const HASH_COMMENT_LANGS = new Set([
  "sh",
  "bash",
  "shell",
  "zsh",
  "py",
  "python",
  "rb",
  "ruby",
  "yaml",
  "yml",
  "toml",
  "make",
  "makefile",
  "dockerfile",
  "ini",
  "conf",
  "r",
  "pl",
  "perl",
]);
const SLASH_COMMENT_LANGS = new Set([
  "js",
  "javascript",
  "ts",
  "typescript",
  "tsx",
  "jsx",
  "rs",
  "rust",
  "go",
  "java",
  "kt",
  "kotlin",
  "swift",
  "c",
  "cpp",
  "cc",
  "h",
  "hpp",
  "cs",
  "scala",
  "dart",
  "php",
  "css",
  "scss",
  "json",
  "jsonc",
]);

function commentPattern(lang: string): string {
  const l = lang.toLowerCase();
  if (HASH_COMMENT_LANGS.has(l)) return "#[^\\n]*";
  if (SLASH_COMMENT_LANGS.has(l)) return "\\/\\/[^\\n]*|\\/\\*[\\s\\S]*?\\*\\/";
  // Unknown or no language: both families, since the fence gave no hint.
  return "\\/\\/[^\\n]*|\\/\\*[\\s\\S]*?\\*\\/|#[^\\n]*";
}

/// Tokenize `code`. `lang` is the fence's info string (may be empty).
export function tokenize(code: string, lang = ""): Token[] {
  const re = new RegExp(
    `(${commentPattern(lang)})|("(?:[^"\\\\\\n]|\\\\.)*"|'(?:[^'\\\\\\n]|\\\\.)*'|\`(?:[^\`\\\\]|\\\\.)*\`)|(\\b\\d+(?:\\.\\d+)?\\b)|([A-Za-z_][A-Za-z0-9_]*)`,
    "g",
  );
  const out: Token[] = [];
  let last = 0;
  const push = (kind: TokenKind, text: string) => {
    if (!text) return;
    const prev = out[out.length - 1];
    if (prev && prev.kind === kind && kind === "plain") prev.text += text;
    else out.push({ kind, text });
  };
  for (const m of code.matchAll(re)) {
    const idx = m.index ?? 0;
    push("plain", code.slice(last, idx));
    if (m[1] !== undefined) push("comment", m[1]);
    else if (m[2] !== undefined) push("string", m[2]);
    else if (m[3] !== undefined) push("number", m[3]);
    else push(KEYWORDS.has(m[4]) ? "keyword" : "plain", m[4]);
    last = idx + m[0].length;
  }
  push("plain", code.slice(last));
  return out;
}
