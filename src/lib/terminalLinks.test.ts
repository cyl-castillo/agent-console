import { describe, expect, it, vi } from "vitest";

import { createTerminalLinkHandler, safeTerminalUrl, type TerminalLinkHint } from "./terminalLinks";

// xterm hands the handler a real MouseEvent; only the cursor position matters.
const at = (x: number, y: number) => ({ clientX: x, clientY: y }) as MouseEvent;

describe("safeTerminalUrl", () => {
  it("accepts http and https", () => {
    expect(safeTerminalUrl("https://example.com/a?b=c#d")).toBe("https://example.com/a?b=c#d");
    expect(safeTerminalUrl("http://localhost:3000/")).toBe("http://localhost:3000/");
  });

  it("trims the surrounding whitespace a CLI may include", () => {
    expect(safeTerminalUrl("  https://example.com  ")).toBe("https://example.com");
  });

  it("refuses schemes a remote process could weaponize inside the webview", () => {
    expect(safeTerminalUrl("javascript:alert(1)")).toBeNull();
    expect(safeTerminalUrl("file:///etc/passwd")).toBeNull();
    expect(safeTerminalUrl("data:text/html,<script>x</script>")).toBeNull();
    expect(safeTerminalUrl("vscode://file/etc/passwd")).toBeNull();
  });

  it("refuses anything that isn't a URL at all", () => {
    expect(safeTerminalUrl("")).toBeNull();
    expect(safeTerminalUrl("example.com")).toBeNull();
    expect(safeTerminalUrl("not a link")).toBeNull();
  });
});

describe("createTerminalLinkHandler", () => {
  function harness(open = vi.fn(async () => {})) {
    const hints: (TerminalLinkHint | null)[] = [];
    const blocked: string[] = [];
    const errors: string[] = [];
    const handler = createTerminalLinkHandler({
      open,
      onHover: (h) => hints.push(h),
      onBlocked: (t) => blocked.push(t),
      onError: (u) => errors.push(u),
    });
    return { handler, open, hints, blocked, errors };
  }

  it("opens a safe link outside the webview", () => {
    const { handler, open } = harness();
    handler.activate(at(0, 0), "https://example.com/docs", {} as never);
    expect(open).toHaveBeenCalledWith("https://example.com/docs");
  });

  it("never opens a non-http link, and says it was blocked", () => {
    const { handler, open, blocked } = harness();
    handler.activate(at(0, 0), "javascript:alert(1)", {} as never);
    expect(open).not.toHaveBeenCalled();
    expect(blocked).toEqual(["javascript:alert(1)"]);
  });

  it("reports a failed open instead of swallowing it", async () => {
    const open = vi.fn(async () => {
      throw new Error("no browser");
    });
    const { handler, errors } = harness(open);
    handler.activate(at(0, 0), "https://example.com", {} as never);
    await vi.waitFor(() => expect(errors).toEqual(["https://example.com"]));
  });

  it("shows the target at the cursor on hover and hides it on leave", () => {
    const { handler, hints } = harness();
    handler.hover!(at(120, 40), "https://example.com", {} as never);
    expect(hints[0]).toEqual({ url: "https://example.com", x: 120, y: 40 });
    handler.leave!(at(120, 40), "https://example.com", {} as never);
    expect(hints[1]).toBeNull();
  });

  it("shows no hint for a link it would refuse to open", () => {
    const { handler, hints } = harness();
    handler.hover!(at(10, 10), "file:///etc/passwd", {} as never);
    expect(hints).toEqual([null]);
  });

  it("clears the hint when the link is activated", () => {
    const { handler, hints } = harness();
    handler.hover!(at(10, 10), "https://example.com", {} as never);
    handler.activate(at(10, 10), "https://example.com", {} as never);
    expect(hints[hints.length - 1]).toBeNull();
  });
});
