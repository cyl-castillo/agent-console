import { beforeEach, describe, expect, it, vi } from "vitest";

const claudeAuthStatus = vi.fn();

// The module pulls in the Tauri IPC layer at load time — stub it so the pure
// decision (which login command to type) is testable without a webview.
vi.mock("../ipc/tauri", () => ({ ipc: { claudeAuthStatus: () => claudeAuthStatus() } }));

import { isAuthError, resolveLoginCmd } from "./loginSession";
import { profileFor } from "../agents/profiles";

const CLAUDE = profileFor("claude");
const CODEX = profileFor("codex");

describe("resolveLoginCmd (fix-login command per installed CLI)", () => {
  // Every case sets its own implementation: mixing mockResolvedValue with a
  // throwing one leaks the rejection past the reset in vitest 4.
  beforeEach(() => {
    claudeAuthStatus.mockReset();
  });

  it("prefers `claude auth login` when the CLI answers the auth probe", async () => {
    claudeAuthStatus.mockImplementation(async () => ({
      loggedIn: false,
      method: null,
      account: null,
    }));
    expect(await resolveLoginCmd(CLAUDE)).toBe("claude auth login");
  });

  it("still prefers it when the probe says we are logged in (creds can be stale)", async () => {
    claudeAuthStatus.mockImplementation(async () => ({
      loggedIn: true,
      method: "claude.ai",
      account: "a@b.c",
    }));
    expect(await resolveLoginCmd(CLAUDE)).toBe("claude auth login");
  });

  it("falls back to plain `claude` when the CLI can't answer (pre-2.1.41)", async () => {
    claudeAuthStatus.mockImplementation(async () => null);
    expect(await resolveLoginCmd(CLAUDE)).toBe("claude");
  });

  it("falls back when the probe itself fails", async () => {
    claudeAuthStatus.mockImplementation(async () => {
      throw new Error("no such command");
    });
    expect(await resolveLoginCmd(CLAUDE)).toBe("claude");
  });

  it("leaves agents without a modern command alone, without probing", async () => {
    expect(await resolveLoginCmd(CODEX)).toBe("codex login");
    expect(claudeAuthStatus).not.toHaveBeenCalled();
  });
});

describe("isAuthError", () => {
  it("matches the backend's structured logged-out hint", () => {
    expect(
      isAuthError(
        "claude exited with status 1: There was an issue with the selected model — `claude auth status` reports you are not logged in; run `claude auth login` in a terminal",
      ),
    ).toBe(true);
  });

  it("ignores unrelated failures", () => {
    expect(isAuthError("claude exited with status 1: file not found")).toBe(false);
    expect(isAuthError(null)).toBe(false);
  });
});
