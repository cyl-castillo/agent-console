import { describe, expect, it } from "vitest";
import { BOOT_TIMEOUT_MS, detect, isBooting, type TermSignals } from "./termSignalsStore";

const boot: TermSignals = { booting: true, bootedAt: 1000 };

describe("terminal signals (F3)", () => {
  it("sees the trust question, the login ask and a missing command through ANSI noise", () => {
    expect(
      detect(
        boot,
        "",
        "\x1b[1mQuick safety check:\x1b[0m Is this a project you created or one you trust?",
        2000,
      ).next.trust,
    ).toBe(2000);
    expect(detect(boot, "", "Do you trust the files in this folder?", 2000).next.trust).toBe(2000);
    expect(detect(boot, "", "Please run /login to authenticate", 2000).next.login).toBe(2000);
    expect(detect(boot, "", "OAuth token has expired.", 2000).next.login).toBe(2000);
    expect(detect(boot, "", "bash: claude: command not found", 2000).next.notFound).toBe(2000);
    expect(
      detect(boot, "", "'claude' is not recognized as an internal or external command", 2000).next
        .notFound,
    ).toBe(2000);
    expect(detect(boot, "", "✓ Read src/retry.ts", 2000).next).toEqual(boot);
  });

  it("the CLI banner ends booting and retires an answered question", () => {
    const asked = detect(boot, "", "Is this a project you created or one you trust?", 2000).next;
    expect(asked.trust).toBe(2000);
    const up = detect(asked, "", "\x1b[38;5;215m✻ Welcome back\x1b[0m claude-opus-5-5", 3000).next;
    expect(up.booting).toBe(false);
    expect(up.ready).toBe(3000);
    expect(up.trust).toBeUndefined();
    // A question raised in the same chunk as the banner stays.
    const both = detect(boot, "", "Welcome to Claude Code\nPlease run /login", 3000).next;
    expect(both.login).toBe(3000);
    expect(both.ready).toBe(3000);
  });

  it("booting is a claim with a deadline", () => {
    expect(isBooting(boot, 1000 + 1)).toBe(true);
    expect(isBooting({ ...boot, ready: 1500 }, 2000)).toBe(false);
    expect(isBooting(boot, 1000 + BOOT_TIMEOUT_MS + 1)).toBe(false);
    expect(isBooting({ booting: false }, 2000)).toBe(false);
  });
});
