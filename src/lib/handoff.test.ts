import { describe, expect, it } from "vitest";
import { handoffPayload, handoffPrompt, quotaOuts, suggestOpencodeModel } from "./handoff";
import type { TimelineTurn } from "../stores/proofStore";

const turn = (over: Partial<TimelineTurn> = {}): TimelineTurn =>
  ({
    turnId: "turn-9",
    termId: "sess-auth",
    caseId: "case-1",
    ts: 1000,
    prompt: "Fix the failing user-auth test and keep the public API",
    files: [
      { status: "M", path: "src/api.ts" },
      { status: "M", path: "tests/auth.test.ts" },
    ],
    filesTruncated: false,
    summary: "Loosened the assertion. All tests pass.",
    summaryTruncated: false,
    failed: false,
    interrupted: false,
    superseded: false,
    tools: [],
    checks: [],
    commits: [],
    verification: {
      seq: 7,
      status: "claim_contradicted",
      reason: '"all tests pass" but npm test exited 1',
      claims: ["all tests pass"],
      checks: { passed: 0, failed: 1, empty: 0, unknown: 0, interrupted: 0 },
      gates: {},
      gateCommands: {},
      coverage: "per_tool",
    },
    ...over,
  }) as unknown as TimelineTurn;

describe("suggestOpencodeModel", () => {
  const m = (id: string, provider: string, name = id) => ({ id, name, provider, variants: [] });
  it("prefers a local open-weight model, then a hosted one, then anything", () => {
    const models = [
      m("openai/gpt-4o", "openai"),
      m("openrouter/qwen3-coder", "openrouter", "Qwen3 Coder"),
      m("ollama/llama3.1", "ollama", "Llama 3.1"),
    ];
    expect(suggestOpencodeModel(models)?.id).toBe("ollama/llama3.1");
    expect(suggestOpencodeModel(models.slice(0, 2))?.id).toBe("openrouter/qwen3-coder");
    expect(suggestOpencodeModel([models[0]])?.id).toBe("openai/gpt-4o");
    expect(suggestOpencodeModel([])).toBeNull();
  });
});

describe("handoffPrompt", () => {
  it("names the reason, the task, the files, the last words and the verdict, and asks for evidence", () => {
    const p = handoffPrompt({
      turn: turn(),
      fromEngine: "claude",
      fromModel: "claude-opus-5-5",
      reason: "budget",
    });
    expect(p).toContain(
      "taking over from claude (claude-opus-5-5), which stopped because its cost cap was reached",
    );
    expect(p).toContain("Fix the failing user-auth test");
    expect(p).toContain("- M src/api.ts");
    expect(p).toContain("Its last words:\nLoosened the assertion. All tests pass.");
    expect(p).toContain('claim_contradicted — "all tests pass" but npm test exited 1');
    expect(p).toContain("report the exit code");
  });
  it("still reads without a witnessed turn, and says quota when that is the reason", () => {
    const p = handoffPrompt({ fromEngine: "codex", reason: "quota" });
    expect(p).toContain("the plan's usage quota ran out");
    expect(p).not.toContain("Files it touched");
    expect(p).toContain("continue");
  });
  it("bounds what the ledger carries", () => {
    const long = turn({
      prompt: "x".repeat(2000),
      files: Array.from({ length: 80 }, (_, i) => ({ status: "M", path: `f${i}` })),
    });
    const payload = handoffPayload({ turn: long, fromEngine: "claude", reason: "budget" });
    expect((payload.prompt as string).length).toBe(501);
    expect((payload.files as string[]).length).toBe(50);
    expect(payload.verification).toBe("claim_contradicted");
  });
});

describe("quotaOuts", () => {
  it("lists sessions whose last turn died on the quota and no hand-over followed", () => {
    const a = turn({ termId: "a", failed: true, error: "rate_limit", ts: 10 });
    const aEarlier = turn({ termId: "a", failed: true, error: "rate_limit", ts: 5 });
    const b = turn({ termId: "b", failed: true, error: "rate_limit", ts: 10 });
    const c = turn({ termId: "c", failed: true, error: "authentication_failed", ts: 10 });
    const d = turn({ termId: "d", failed: false, ts: 10 });
    const events = [
      { kind: "model_switch", termId: "b", ts: 11, payload: { reason: "quota" } },
      { kind: "model_switch", termId: "a", ts: 3, payload: { reason: "quota" } },
    ];
    const out = quotaOuts([aEarlier, a, b, c, d], events);
    expect(out.map((t) => t.termId)).toEqual(["a"]);
  });
});
