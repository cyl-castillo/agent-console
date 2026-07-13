import { describe, expect, it } from "vitest";

import { profileFor, isSafeSessionId } from "./profiles";

const UUID = "0198c2f2-8a25-7e11-b3a4-8d6a3d5c2f10";

describe("CODEX buildLaunch", () => {
  const codex = profileFor("codex");

  it("resumes by id when the hook captured one", () => {
    const { cmd, note } = codex.buildLaunch({ agentSessionId: UUID, hasScrollback: true });
    expect(cmd).toBe(`codex resume ${UUID}`);
    expect(note).toBe("auto-resuming");
  });

  it("starts fresh without an id — and never uses global --last", () => {
    const { cmd, note } = codex.buildLaunch({ hasScrollback: true });
    expect(cmd).toBe("codex");
    expect(cmd).not.toContain("--last");
    expect(note).toContain("no session id");
  });

  it("encodes the chosen effort as a config override, on fresh and resume", () => {
    expect(codex.buildLaunch({ model: "high", hasScrollback: false }).cmd)
      .toBe("codex -c model_reasoning_effort=high");
    expect(codex.buildLaunch({ agentSessionId: UUID, model: "low", hasScrollback: true }).cmd)
      .toBe(`codex resume ${UUID} -c model_reasoning_effort=low`);
  });

  it("rejects a shell-unsafe session id (falls back to fresh)", () => {
    const { cmd } = codex.buildLaunch({
      agentSessionId: "bad; rm -rf /",
      hasScrollback: true,
    });
    expect(cmd).toBe("codex");
  });
});

describe("CLAUDE buildLaunch", () => {
  const claude = profileFor("claude");

  it("resumes by id with model flag", () => {
    const { cmd } = claude.buildLaunch({ agentSessionId: UUID, model: "opus", hasScrollback: true });
    expect(cmd).toBe(`claude --resume ${UUID} --model opus`);
  });

  it("rejects a shell-unsafe session id (falls back to fresh)", () => {
    const { cmd } = claude.buildLaunch({ agentSessionId: "$(evil)", hasScrollback: true });
    expect(cmd).toBe("claude");
  });
});

describe("isSafeSessionId", () => {
  it("accepts UUIDs and rejects shell metacharacters", () => {
    expect(isSafeSessionId(UUID)).toBe(true);
    expect(isSafeSessionId("thread-name_1.2")).toBe(true);
    expect(isSafeSessionId("a b")).toBe(false);
    expect(isSafeSessionId("x;y")).toBe(false);
    expect(isSafeSessionId("")).toBe(false);
    expect(isSafeSessionId(undefined)).toBe(false);
  });
});
