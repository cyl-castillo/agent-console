// A project with thirty sessions: long names, branches, a few mid-turn —
// the sidebar at the scale a week of work reaches. Recency is scrambled on
// purpose (session k was last active (k · 7) mod 30 "slots" ago), so a list
// that keeps creation order shows up in the F1 gate.
import type { PersistedSession, ProofEvent } from "../../types/domain";
import { NOW, ROOT } from "./defaults";

const NAMES = [
  "Migrate the billing webhooks to the new idempotency layer",
  "retry path regression",
  "Explain why the nightly job double-counts refunds",
  "shell 4",
  "Add rate limiting to the public search endpoint",
  "fix auth test",
  "Write the ADR for moving sessions to SQLite",
  "Investigate the flaky e2e on Windows runners",
  "shell 9",
  "Replace moment with date-fns across the dashboard",
];

export const MANY_COUNT = 30;
export const manyId = (k: number) => `sess-m${String(k).padStart(2, "0")}`;
/// Slots ago (one slot = 10 min) the session was last active.
export const manySlot = (k: number) => (k * 7) % MANY_COUNT;

export const manySessions: PersistedSession[] = Array.from({ length: MANY_COUNT }, (_, i) => {
  const k = i + 1;
  const s: PersistedSession = {
    id: manyId(k),
    name: `${NAMES[i % NAMES.length]}${k > NAMES.length ? ` (${k})` : ""}`,
    cwd: ROOT,
    createdAtMs: NOW - 86_400_000 - k * 3_600_000,
    lastActiveMs: NOW - manySlot(k) * 600_000,
    scrollback: "",
    agent: k % 4 === 0 ? "codex" : "claude",
  };
  if (k % 3 === 0) {
    s.worktree = {
      path: `/home/demo/.local/share/agent-console/worktrees/demo-playground/t${k}`,
      branch: `ac/feature-branch-number-${k}`,
      baseBranch: "main",
    };
  }
  return s;
});

/// Sessions 1–3 are mid-turn (a prompt with no close); 4–8 finished a turn.
export const manyEvents: ProofEvent[] = (() => {
  const out: ProofEvent[] = [];
  let seq = 0;
  const ev = (
    ts: number,
    kind: string,
    termId: string,
    turnId: string,
    actor: string,
    payload: Record<string, unknown>,
  ) =>
    out.push({
      seq: ++seq,
      ts,
      caseId: `case-${termId}`,
      turnId,
      kind,
      termId,
      actor,
      payload,
      prevHash: "",
      hash: "",
    });
  for (let k = 1; k <= 8; k++) {
    const id = manyId(k);
    ev(NOW - 120_000, "prompt", id, `turn-${id}`, "human", { prompt: `Work on ${NAMES[k - 1]}` });
    if (k > 3)
      ev(NOW - 60_000, "turn_end", id, `turn-${id}`, "agent", {
        summary: `Done with ${NAMES[k - 1].toLowerCase()}.`,
        filesChanged: [],
      });
  }
  return out;
})();
