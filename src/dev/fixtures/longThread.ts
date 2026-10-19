// One session with two hundred turns: the thread at the scale a long day of
// pairing reaches. The F6 gate reads this story for long tasks and tab stops
// — a thread that renders every turn shows up here.
import type { PersistedSession, ProofEvent } from "../../types/domain";
import { NOW, ROOT } from "./defaults";

export const LONG_ID = "sess-long";
export const LONG_TURNS = 200;

export const longSession: PersistedSession = {
  id: LONG_ID,
  name: "long pairing day",
  cwd: ROOT,
  createdAtMs: NOW - 10 * 3_600_000,
  lastActiveMs: NOW - 30_000,
  scrollback: "",
  agent: "claude",
  agentSessionId: "9c1d2e3f-4a5b-4c6d-8e7f-0a1b2c3d4e5f",
};

const PROMPTS = [
  "Fix the failing retry test",
  "Add a regression test for the backoff ceiling",
  "Refactor retry.ts so the ceiling is configurable",
  "Explain why the nightly job double-counts refunds",
  "Write the ADR for moving sessions to SQLite",
];

export const longEvents: ProofEvent[] = (() => {
  const out: ProofEvent[] = [];
  let seq = 0;
  const ev = (
    ts: number,
    kind: string,
    turnId: string,
    actor: string,
    payload: Record<string, unknown>,
  ) =>
    out.push({
      seq: ++seq,
      ts,
      caseId: "case-long",
      turnId,
      kind,
      termId: LONG_ID,
      sessionId: longSession.agentSessionId,
      actor,
      payload,
      prevHash: "",
      hash: "",
    });
  for (let k = 1; k <= LONG_TURNS; k++) {
    const turnId = `turn-long-${String(k).padStart(3, "0")}`;
    const t0 = NOW - (LONG_TURNS - k + 1) * 150_000;
    ev(t0, "prompt", turnId, "human", { prompt: `${PROMPTS[k % PROMPTS.length]} (#${k})` });
    ev(t0 + 10_000, "tool_result", turnId, "agent", {
      tool: "Read",
      command: undefined,
      excerpt: "src/retry.ts",
    });
    ev(t0 + 20_000, "tool_result", turnId, "agent", {
      tool: "Bash",
      command: "npm test -- retry",
      exitCode: k % 17 === 0 ? 1 : 0,
      failed: k % 17 === 0,
    });
    if (k % 3 === 0)
      ev(t0 + 25_000, "tool_result", turnId, "agent", { tool: "Edit", excerpt: "src/retry.ts" });
    if (k < LONG_TURNS)
      ev(t0 + 60_000, "turn_end", turnId, "agent", {
        summary: `Turn ${k}: the retry ceiling now comes from **RetryOptions**; \`npm test -- retry\` is green.`,
        filesChanged: k % 3 === 0 ? [{ status: "M", path: "src/retry.ts" }] : [],
      });
  }
  return out;
})();
