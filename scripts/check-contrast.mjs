/* global process */
// WCAG 2.2 contrast gate over the DTCG tokens: every text tier must reach
// 4.5:1 on every surface it is used on, in both modes; status colours used
// as text (ok/danger/warn/purple/accent) must reach 4.5:1 on the base
// surfaces too. Prints the full matrix; exits 1 on the first failure.
import { readFileSync } from "node:fs";

const doc = JSON.parse(readFileSync(process.argv[2] ?? "tokens/console.tokens.json", "utf8"));
const modes = ["dark", "light"];
const surfaces = ["bg", "bg-elev-1", "bg-elev-2"]; // elev-3 is floating chrome: text/dim only
const inks = ["text", "text-dim", "text-muted", "text-faint"];
const status = ["accent", "ok", "danger", "warn", "purple"];

const hex = (name, mode) => doc.color[name].$extensions.mode[mode].hex;
const lin = (c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const lum = (h) => {
  const [r, g, b] = [1, 3, 5].map((i) => lin(parseInt(h.slice(i, i + 2), 16) / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [l1, l2] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
};

let failures = 0;
for (const mode of modes) {
  console.log(`\n${mode}`);
  console.log(`${"".padEnd(12)}${surfaces.map((s) => s.padStart(11)).join("")}`);
  for (const ink of [...inks, ...status]) {
    const cells = surfaces.map((s) => {
      const r = ratio(hex(ink, mode), hex(s, mode));
      const ok = r >= 4.5;
      if (!ok) failures += 1;
      return `${r.toFixed(2).padStart(9)}${ok ? "  " : " ✗"}`;
    });
    console.log(`${ink.padEnd(12)}${cells.join("")}`);
  }
}
console.log(failures ? `\n${failures} cell(s) below 4.5:1` : "\nall text tokens ≥ 4.5:1");
process.exit(failures ? 1 : 0);
