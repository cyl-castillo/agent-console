// Foundations: the specimens the redesign decisions are made against —
// type candidates at the proposed scale, every colour token with its live
// contrast ratio, and the accent candidates applied to real controls.
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CSSProperties } from "react";
import tokens from "../../tokens/console.tokens.json";
import { useThemeStore } from "../stores/themeStore";

const meta: Meta = { title: "Foundations", parameters: { layout: "fullscreen" } };
export default meta;

type Mode = "dark" | "light";
const colorNames = Object.keys(tokens.color);
type ColorToken = { $extensions: { mode: Record<Mode, { hex: string }> } };
const hexOf = (name: string, mode: Mode) =>
  (tokens.color as Record<string, ColorToken>)[name].$extensions.mode[mode].hex;

const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const lum = (h: string) => {
  const [r, g, b] = [1, 3, 5].map((i) => lin(parseInt(h.slice(i, i + 2), 16) / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a: string, b: string) => {
  const [l1, l2] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
};

const page: CSSProperties = {
  padding: 24,
  fontFamily: "var(--font-ui)",
  color: "var(--text)",
  background: "var(--bg)",
  minHeight: "100vh",
  overflow: "auto",
};
const h2: CSSProperties = {
  fontSize: 13,
  fontWeight: 600,
  margin: "24px 0 8px",
  color: "var(--text-dim)",
};

function withTheme(mode: Mode): StoryObj["loaders"] {
  return [
    async () => {
      useThemeStore.getState().setTheme(mode);
      return {};
    },
  ];
}
const ready = async () => {
  await new Promise((r) => setTimeout(r, 200));
  await document.fonts.ready;
  document.body.setAttribute("data-story-ready", "1");
};

const SCALE: [string, number, number][] = [
  ["display", 20, 600],
  ["title", 16, 600],
  ["lead", 14, 400],
  ["body", 13, 400],
  ["small", 12, 400],
  ["caption", 11, 400],
];
const SANS = [
  ["Inter Variable", "Inter 4 (opsz) — propuesta"],
  ["Geist Variable", "Geist Sans — alternativa documentada"],
];
const MONO = [
  ["JetBrains Mono Variable", "JetBrains Mono — propuesta"],
  ["Geist Mono Variable", "Geist Mono — alternativa"],
];

function Typography() {
  return (
    <div style={page}>
      <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>Tipografía</h1>
      <p style={{ color: "var(--text-dim)", fontSize: 13, maxWidth: 640 }}>
        Escala propuesta en rem (mínimo duro 11 px). Sans para el chrome; mono solo para código,
        paths, hashes, diffs, terminal y cifras tabulares.
      </p>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 32 }}>
        {SANS.map(([family, label]) => (
          <section key={family} style={{ fontFamily: `"${family}", sans-serif` }}>
            <h2 style={h2}>{label}</h2>
            {SCALE.map(([name, px, weight]) => (
              <div
                key={name}
                style={{ display: "flex", alignItems: "baseline", gap: 12, margin: "6px 0" }}
              >
                <span
                  style={{
                    width: 64,
                    fontSize: 11,
                    color: "var(--text-faint)",
                    fontFamily: "var(--font-mono)",
                  }}
                >
                  {name} {px}
                </span>
                <span
                  style={{ fontSize: px, fontWeight: weight, fontVariantNumeric: "tabular-nums" }}
                >
                  El agente editó src/retry.ts y corrió 14 tests en 4,1 s — 0 fallos.
                </span>
              </div>
            ))}
          </section>
        ))}
        {MONO.map(([family, label]) => (
          <section key={family} style={{ fontFamily: `"${family}", monospace` }}>
            <h2 style={h2}>{label}</h2>
            <pre style={{ margin: 0, fontSize: 13, lineHeight: 1.5, whiteSpace: "pre-wrap" }}>
              {`$ npm test -- --run tests/retry.test.ts
✓ backoff ceiling is 8s (12 ms)
✓ gives up after 5 attempts (3 ms)
a1b2c3d4  src/retry.ts  +18 −6
0Oo 1Il| {}[]() => != === 0.065 1,234,567.89`}
            </pre>
          </section>
        ))}
      </div>
    </div>
  );
}

function Colors({ mode }: { mode: Mode }) {
  const surfaces = ["bg", "bg-elev-1", "bg-elev-2", "bg-elev-3"];
  const inks = colorNames.filter((n) => !surfaces.includes(n) && n !== "border");
  return (
    <div style={page}>
      <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>Color — {mode}</h1>
      <p style={{ color: "var(--text-dim)", fontSize: 13 }}>
        Ratio de contraste de cada tinta sobre cada superficie (WCAG 2.2; AA texto = 4,5:1).
      </p>
      <table
        style={{ borderCollapse: "collapse", fontSize: 12, fontVariantNumeric: "tabular-nums" }}
      >
        <thead>
          <tr>
            <th style={{ textAlign: "left", padding: 6 }}>token</th>
            {surfaces.map((s) => (
              <th key={s} style={{ padding: 6, textAlign: "right" }}>
                {s}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {inks.map((ink) => (
            <tr key={ink}>
              <td style={{ padding: 6, fontFamily: "var(--font-mono)" }}>
                <span
                  style={{
                    display: "inline-block",
                    width: 12,
                    height: 12,
                    background: hexOf(ink, mode),
                    borderRadius: 3,
                    marginRight: 8,
                    verticalAlign: -1,
                  }}
                />
                {ink} <span style={{ color: "var(--text-faint)" }}>{hexOf(ink, mode)}</span>
              </td>
              {surfaces.map((s) => {
                const r = ratio(hexOf(ink, mode), hexOf(s, mode));
                return (
                  <td
                    key={s}
                    style={{
                      padding: 6,
                      textAlign: "right",
                      background: hexOf(s, mode),
                      color: hexOf(ink, mode),
                    }}
                  >
                    {r.toFixed(2)} {r >= 4.5 ? "" : "✗"}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const ACCENTS: [string, string, string][] = [
  ["actual (azul)", "#6aa9ff", "#2563eb"],
  ["verdigris (propuesta)", "#5fc2c0", "#0f7f7d"],
  ["lavanda (Linear)", "#8b8cf0", "#5e6ad2"],
  ["terracota (Claude)", "#e08a5c", "#c96442"],
  ["naranja quemado", "#f0a060", "#b35a1a"],
];

function Accents({ mode }: { mode: Mode }) {
  const surface = hexOf("bg-elev-1", mode);
  return (
    <div style={page}>
      <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>Acento — {mode}</h1>
      <p style={{ color: "var(--text-dim)", fontSize: 13, maxWidth: 640 }}>
        Un solo acento con relleno sólido (botón primario, selección, anillo de foco). No puede
        competir con ámbar = te necesita, verde = terminó, rojo = falló. Elección de diseño, no de
        evidencia: se decide acá.
      </p>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
          gap: 16,
        }}
      >
        {ACCENTS.map(([label, dark, light]) => {
          const accent = mode === "dark" ? dark : light;
          const onAccent =
            ratio("#ffffff", accent) >= ratio("#0d0f12", accent) ? "#ffffff" : "#0d0f12";
          return (
            <section
              key={label}
              style={{
                background: surface,
                border: "1px solid var(--border)",
                borderRadius: 8,
                padding: 14,
              }}
            >
              <h2 style={{ ...h2, margin: "0 0 10px" }}>{label}</h2>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                <button
                  style={{
                    background: accent,
                    color: onAccent,
                    border: 0,
                    borderRadius: 6,
                    padding: "6px 12px",
                    fontSize: 13,
                    fontWeight: 500,
                  }}
                >
                  Approve once
                </button>
                <button
                  style={{
                    background: "transparent",
                    color: "var(--text)",
                    border: "1px solid var(--border)",
                    borderRadius: 6,
                    padding: "6px 12px",
                    fontSize: 13,
                    outline: `2px solid ${accent}`,
                    outlineOffset: 2,
                  }}
                >
                  focus
                </button>
                <span
                  style={{
                    background: `color-mix(in srgb, ${accent} 15%, transparent)`,
                    color: accent,
                    borderRadius: 999,
                    padding: "2px 8px",
                    fontSize: 12,
                  }}
                >
                  working…
                </span>
              </div>
              <div
                style={{
                  marginTop: 10,
                  fontSize: 12,
                  color: "var(--text-dim)",
                  display: "flex",
                  gap: 10,
                }}
              >
                <span style={{ color: "#f0b34a" }}>● te necesita</span>
                <span style={{ color: "#6cd16c" }}>● terminó</span>
                <span style={{ color: "#ff6a6a" }}>● falló</span>
              </div>
              <div
                style={{
                  marginTop: 8,
                  fontSize: 11,
                  color: "var(--text-faint)",
                  fontFamily: "var(--font-mono)",
                }}
              >
                {accent} · texto {ratio(accent, surface).toFixed(2)}:1 · relleno{" "}
                {ratio(onAccent, accent).toFixed(2)}:1
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

export const TypographyDark: StoryObj = {
  render: () => <Typography />,
  loaders: withTheme("dark"),
  play: ready,
};
export const TypographyLight: StoryObj = {
  render: () => <Typography />,
  loaders: withTheme("light"),
  play: ready,
};
export const ColorDark: StoryObj = {
  render: () => <Colors mode="dark" />,
  loaders: withTheme("dark"),
  play: ready,
};
export const ColorLight: StoryObj = {
  render: () => <Colors mode="light" />,
  loaders: withTheme("light"),
  play: ready,
};
export const AccentDark: StoryObj = {
  render: () => <Accents mode="dark" />,
  loaders: withTheme("dark"),
  play: ready,
};
export const AccentLight: StoryObj = {
  render: () => <Accents mode="light" />,
  loaders: withTheme("light"),
  play: ready,
};
