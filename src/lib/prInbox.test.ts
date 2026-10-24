import { describe, expect, it } from "vitest";
import {
  autofixPrompt,
  autofixedAt,
  ciLabel,
  evidenceLabel,
  mergeGate,
  prNeedsYou,
  prTermIds,
  AUTOFIX_BODY_CAP,
} from "./prInbox";
import type { PrOverview, ProofEvent } from "../types/domain";

const ov = (over: Partial<PrOverview["pr"]> = {}, ev: Partial<PrOverview["evidence"]> = {}) =>
  ({
    branch: "feat/x",
    gh: "ok",
    pr: {
      number: 41,
      url: "https://github.com/demo/demo-playground/pull/41",
      title: "Retry path",
      state: "OPEN",
      isDraft: false,
      headRef: "feat/x",
      baseRef: "main",
      headSha: "abc123",
      mergeState: "BLOCKED",
      reviewDecision: "",
      checks: [
        { name: "frontend", status: "failure" },
        { name: "backend", status: "success" },
      ],
      ci: "red",
      comments: [
        {
          id: 1,
          author: "reviewer-bot",
          bot: true,
          kind: "inline",
          body: "<!-- hidden -->\n**P1**  Preserve rules\n\nWhen the project already has rules…",
          path: "src/policy.rs",
          line: 439,
          createdAt: "2026-10-24T00:37:03Z",
        },
        {
          id: 2,
          author: "carla",
          bot: false,
          kind: "review",
          body: "Looks right, one nit.",
          createdAt: "2026-10-24T01:00:00Z",
        },
      ],
      ...over,
    },
    evidence: {
      branch: "feat/x",
      base: "main",
      commits: [
        {
          sha: "abc123",
          subject: "Fix",
          caseId: "term:sess-auth",
          turnId: "turn-001",
          termId: "sess-auth",
          actor: "agent",
          verification: "verified",
        },
        { sha: "def456", subject: "Manual tweak", verification: "unbound" },
      ],
      verified: 1,
      accepted: 0,
      unverified: 0,
      contradicted: 0,
      unbound: 1,
      allVerified: false,
      ...ev,
    },
  }) as PrOverview;

const autofixLine = (number: number, headSha: string): ProofEvent => ({
  seq: 1,
  ts: 1,
  caseId: "term:sess-auth",
  kind: "pr_autofix",
  actor: "human",
  payload: { number, headSha, commentIds: [1, 2] },
  prevHash: "",
  hash: "",
});

describe("prNeedsYou", () => {
  it("red CI is a decision whatever else is true", () => {
    expect(prNeedsYou(ov(), [])).toBe("ci_red");
  });
  it("fresh comments are a decision; once handed to the agent for this head they are not", () => {
    expect(prNeedsYou(ov({ ci: "pending" }), [])).toBe("comments");
    expect(prNeedsYou(ov({ ci: "pending" }), [autofixLine(41, "abc123")])).toBeNull();
    expect(prNeedsYou(ov({ ci: "pending" }), [autofixLine(41, "newpush")])).toBe("comments");
  });
  it("green and open is the merge word; running CI alone is only a chip", () => {
    expect(prNeedsYou(ov({ ci: "green", comments: [] }), [])).toBe("ready");
    expect(prNeedsYou(ov({ ci: "pending", comments: [] }), [])).toBeNull();
  });
  it("merged, closed, draft or dismissed PRs ask nothing", () => {
    expect(prNeedsYou(ov({ state: "MERGED" }), [])).toBeNull();
    expect(prNeedsYou(ov({ isDraft: true }), [])).toBeNull();
    expect(prNeedsYou(ov(), [], ["41:abc123"])).toBeNull();
    expect(prNeedsYou(null, [])).toBeNull();
  });
});

describe("mergeGate", () => {
  it("names what holds the automatic gate", () => {
    const g = mergeGate(ov());
    expect(g.gate).toBe("human");
    expect(g.reasons).toEqual(["1 commit the ledger never saw"]);
  });
  it("is by evidence only when every commit sits on a verified or accepted turn", () => {
    const g = mergeGate(
      ov(
        {},
        {
          commits: [
            { sha: "a", subject: "A", turnId: "t", termId: "s", verification: "verified" },
            { sha: "b", subject: "B", turnId: "u", termId: "s", verification: "accepted" },
          ],
          verified: 1,
          accepted: 1,
          unbound: 0,
          allVerified: true,
        },
      ),
    );
    expect(g).toEqual({ gate: "verified", reasons: [] });
  });
  it("an empty branch is never verified", () => {
    const g = mergeGate(ov({}, { commits: [], verified: 0, unbound: 0, allVerified: false }));
    expect(g.gate).toBe("human");
    expect(g.reasons[0]).toMatch(/no commits/);
  });
});

describe("autofixPrompt", () => {
  it("lists each comment with file and line, strips hidden markup, ends on the gate", () => {
    const p = autofixPrompt(ov());
    expect(p.startsWith("CI is red (frontend). Address the review comments on PR #41")).toBe(true);
    expect(p).toContain(
      "1. src/policy.rs:439 — @reviewer-bot: **P1** Preserve rules When the project",
    );
    expect(p).not.toContain("hidden");
    expect(p).toContain("2. review — @carla: Looks right, one nit.");
    expect(p).toContain("do not say it passes unless the run exits 0");
  });
  it("caps long bodies and says how many comments stay on the page", () => {
    const many = Array.from({ length: 25 }, (_, i) => ({
      id: i,
      author: "r",
      bot: false,
      kind: "inline",
      body: "x".repeat(1000),
      path: "a.ts",
      line: i + 1,
      createdAt: "",
    }));
    const p = autofixPrompt(ov({ ci: "green", comments: many }));
    expect(p).not.toMatch(/^CI is red/);
    expect(p).toContain("(5 more on the PR page.)");
    const first = p.split("\n")[1];
    expect(first.length).toBeLessThan(AUTOFIX_BODY_CAP + 40);
    expect(first.endsWith("…")).toBe(true);
  });
  it("is empty without a PR", () => {
    expect(autofixPrompt({ ...ov(), pr: undefined })).toBe("");
  });
});

describe("labels", () => {
  it("reads the CI and the evidence in one word each", () => {
    expect(ciLabel("green")).toBe("CI green");
    expect(ciLabel("pending")).toBe("CI running");
    expect(ciLabel("none")).toBe("no CI");
    expect(evidenceLabel(ov())).toBe("verified 1/2");
    expect([...prTermIds(ov())]).toEqual(["sess-auth"]);
    expect(autofixedAt([autofixLine(41, "abc123")], 41, "abc123")).toBe(true);
  });
});
