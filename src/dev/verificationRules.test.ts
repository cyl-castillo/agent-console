import { describe, expect, it } from "vitest";

import { recomputeVerification } from "../../.github/testigo/verification-rules.mjs";
import rulesSource from "../../.github/testigo/verification-rules.mjs?raw";
import cases from "../../src-tauri/fixtures/verification/cases.json?raw";
import verifierHtml from "../../src-tauri/resources/testigo-verifier.html?raw";

type Case = {
  name: string;
  events: { kind: string; payload?: unknown }[];
  expected: { status: string; coverage: string; claims: number };
};

describe("verification rules (JS port)", () => {
  it("agrees with every shared vector (the Rust rules run the same file)", () => {
    const doc = JSON.parse(cases) as { cases: Case[] };
    for (const c of doc.cases) {
      const r = recomputeVerification(c.events.map((e, i) => ({ seq: i + 1, ...e })));
      const live = r.claims.filter((x) => !x.negated).length;
      expect({ name: c.name, status: r.status, coverage: r.coverage, claims: live }).toEqual({
        name: c.name,
        status: c.expected.status,
        coverage: c.expected.coverage,
        claims: c.expected.claims,
      });
    }
  });

  it("is embedded byte-identical in the standalone verifier page", () => {
    const between = (s: string) => {
      const b = s.indexOf("/* verification-rules:begin */");
      const e = s.indexOf("/* verification-rules:end */");
      expect(b).toBeGreaterThanOrEqual(0);
      expect(e).toBeGreaterThan(b);
      return s.slice(b, e);
    };
    const fromModule = between(rulesSource)
      .replace(/export const /g, "const ")
      .replace(/export function /g, "function ");
    expect(between(verifierHtml)).toBe(fromModule);
  });
});
