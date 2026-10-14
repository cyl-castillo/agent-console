/* global process */
// Static dead-class scan: every class selector in src/styles/global.css whose
// class name appears in no TS/TSX source (className strings, template
// literals, cva variants) and in no other stylesheet-composed rule is dead —
// its panel was rewritten or removed. Reports (default) or prunes (--prune)
// whole rules whose every selector is dead. Third-party DOM (xterm, cmdk,
// Base UI data attributes) is allowlisted.
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const CSS = "src/styles/global.css";
const prune = process.argv.includes("--prune");
// Bisect helpers: prune only dead rules [skip, skip+limit).
const arg = (name, dflt) => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? Number(process.argv[i + 1]) : dflt;
};
const skip = arg("--skip", 0);
const limit = arg("--limit", Infinity);
const ALLOW = /^(xterm|cmdk|lucide|composer-|sb-|tauri)/;

function walk(dir, out = []) {
  for (const f of readdirSync(dir)) {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.(tsx?|mjs|html)$/.test(f) && !/\.test\.tsx?$/.test(f)) out.push(p);
  }
  return out;
}
const sources = [...walk("src"), "index.html"].map((p) => readFileSync(p, "utf8")).join("\n");

const css = readFileSync(CSS, "utf8");
// Split into top-level rules (selector { ... }); @media blocks are scanned inside.
const rules = [];
const re = /([^{}]+)\{([^{}]*)\}/g;
function scan(text, offset) {
  let m;
  while ((m = re.exec(text))) {
    const sel = m[1].trim();
    if (sel.startsWith("@")) {
      // nested block: recurse into its body
      continue;
    }
    rules.push({ sel, start: offset + m.index, end: offset + m.index + m[0].length });
  }
}
scan(css, 0);

const classesOf = (sel) => [...sel.matchAll(/\.([A-Za-z_][\w-]*)/g)].map((x) => x[1]);
// A class is alive when its full name appears in a source, or when a source
// builds it from a template (`task-${status}` keeps every `.task-*` alive).
const prefixes = new Set([...sources.matchAll(/["'` ]([a-z][\w-]*-)\$\{/g)].map((m) => m[1]));
const used = (cls) =>
  ALLOW.test(cls) || sources.includes(cls) || [...prefixes].some((pre) => cls.startsWith(pre));

let deadRules = [];
for (const r of rules) {
  const sels = r.sel.split(",").map((s) => s.trim());
  const deadSel = sels.filter((s) => {
    const cls = classesOf(s);
    if (cls.length === 0) return false; // element/attr rules: keep
    return cls.some((c) => !used(c)); // any unknown class makes the selector unmatched
  });
  if (deadSel.length === sels.length) deadRules.push(r);
}

const lines = (text, i) => text.slice(0, i).split("\n").length;
console.log(`${rules.length} rules, ${deadRules.length} dead`);
for (const r of deadRules)
  console.log(`  L${lines(css, r.start)}  ${r.sel.replace(/\s+/g, " ").slice(0, 90)}`);

if (prune && deadRules.length) {
  deadRules = deadRules.slice(skip, skip + limit);
  let out = css;
  for (const r of [...deadRules].sort((a, b) => b.start - a.start)) {
    out = out.slice(0, r.start) + out.slice(r.end);
  }
  out = out.replace(/\n{3,}/g, "\n\n");
  writeFileSync(CSS, out);
  console.log(`pruned ${deadRules.length} rules; ${out.split("\n").length} lines left`);
}
