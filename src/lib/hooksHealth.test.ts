import { describe, expect, it } from "vitest";

import {
  assessHooksHealth,
  EMPTY_SIGNAL,
  formatAge,
  noteEvent,
  noteInput,
  STALE_AFTER_MS,
  STALE_SUBMITS,
  suspiciousTerms,
  type TermSignal,
} from "./hooksHealth";

const T0 = 1_000_000;

function submitted(n: number, firstAt = T0): TermSignal {
  return { ...EMPTY_SIGNAL, unansweredSubmits: n, firstUnansweredMs: firstAt };
}

describe("noteInput (Enter after typing = a submission)", () => {
  it("counts Enter only when something printable was typed since the last Enter", () => {
    let s = noteInput(EMPTY_SIGNAL, "\r", T0);
    expect(s.unansweredSubmits).toBe(0);
    s = noteInput(s, "fix the test", T0);
    expect(s.typedSinceEnter).toBe(12);
    s = noteInput(s, "\r", T0 + 10);
    expect(s.unansweredSubmits).toBe(1);
    expect(s.firstUnansweredMs).toBe(T0 + 10);
    expect(s.typedSinceEnter).toBe(0);
    // A second Enter on an empty line is not a second submission.
    s = noteInput(s, "\r", T0 + 20);
    expect(s.unansweredSubmits).toBe(1);
  });

  it("keeps the first pending timestamp across later submissions", () => {
    let s = noteInput(EMPTY_SIGNAL, "a\r", T0);
    s = noteInput(s, "b\r", T0 + 5000);
    expect(s.unansweredSubmits).toBe(2);
    expect(s.firstUnansweredMs).toBe(T0);
  });

  it("ignores escape sequences and control characters", () => {
    let s = noteInput(EMPTY_SIGNAL, "\x1b[A", T0);
    s = noteInput(s, "\x03", T0);
    expect(s.typedSinceEnter).toBe(0);
    s = noteInput(s, "\r", T0);
    expect(s.unansweredSubmits).toBe(0);
  });

  it("handles a pasted multi-line chunk as one input stream", () => {
    const s = noteInput(EMPTY_SIGNAL, "line one\rline two\r", T0);
    expect(s.unansweredSubmits).toBe(2);
  });
});

describe("noteEvent", () => {
  it("answers everything pending and stamps the event time", () => {
    const s = noteEvent(submitted(3), T0 + 1);
    expect(s.unansweredSubmits).toBe(0);
    expect(s.firstUnansweredMs).toBe(0);
    expect(s.lastEventMs).toBe(T0 + 1);
  });
});

describe("suspiciousTerms", () => {
  it("needs both the submit count and the age", () => {
    const now = T0 + STALE_AFTER_MS;
    const perTerm = {
      fresh: submitted(STALE_SUBMITS, now - 1000),
      few: submitted(STALE_SUBMITS - 1, T0),
      stale: submitted(STALE_SUBMITS, T0),
      answered: noteEvent(submitted(5), now),
    };
    expect(suspiciousTerms(perTerm, now)).toEqual(["stale"]);
  });
});

describe("assessHooksHealth", () => {
  const now = T0 + STALE_AFTER_MS;

  it("is off when hooks are not installed, whatever the signals say", () => {
    expect(
      assessHooksHealth({
        installed: false,
        lastEventMs: now,
        perTerm: { t: submitted(9) },
        liveClaudeTerms: new Set(["t"]),
        now,
      }),
    ).toEqual({ kind: "off" });
  });

  it("is silent before any event and without suspicion", () => {
    expect(
      assessHooksHealth({
        installed: true,
        lastEventMs: null,
        perTerm: {},
        liveClaudeTerms: new Set(),
        now,
      }),
    ).toEqual({ kind: "silent" });
  });

  it("is stale only for suspicious terminals that actually run Claude", () => {
    const perTerm = { claudeTerm: submitted(2), shellTerm: submitted(2) };
    const v = assessHooksHealth({
      installed: true,
      lastEventMs: null,
      perTerm,
      liveClaudeTerms: new Set(["claudeTerm"]),
      now,
    });
    expect(v).toEqual({ kind: "stale", termIds: ["claudeTerm"], lastEventMs: null });
    // A plain shell swallowing Enters proves nothing about hooks.
    expect(
      assessHooksHealth({
        installed: true,
        lastEventMs: null,
        perTerm,
        liveClaudeTerms: new Set(),
        now,
      }),
    ).toEqual({ kind: "silent" });
  });

  it("stale wins over ok: one working terminal doesn't excuse a silent one", () => {
    const v = assessHooksHealth({
      installed: true,
      lastEventMs: now - 5,
      perTerm: { good: noteEvent(EMPTY_SIGNAL, now - 5), bad: submitted(2) },
      liveClaudeTerms: new Set(["good", "bad"]),
      now,
    });
    expect(v.kind).toBe("stale");
    if (v.kind === "stale") expect(v.termIds).toEqual(["bad"]);
  });

  it("is ok once events flow and nothing is pending", () => {
    expect(
      assessHooksHealth({
        installed: true,
        lastEventMs: now - 12_000,
        perTerm: { t: noteEvent(submitted(2), now - 12_000) },
        liveClaudeTerms: new Set(["t"]),
        now,
      }),
    ).toEqual({ kind: "ok", lastEventMs: now - 12_000 });
  });
});

describe("formatAge", () => {
  it("picks the largest useful unit", () => {
    expect(formatAge(0)).toBe("0s");
    expect(formatAge(12_400)).toBe("12s");
    expect(formatAge(3 * 60_000)).toBe("3m");
    expect(formatAge(2 * 3_600_000)).toBe("2h");
  });
});
