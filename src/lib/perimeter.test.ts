import { describe, expect, it } from "vitest";
import { hostAllowed, hostsIn, hostsOutside, parseHosts } from "./perimeter";
import type { ApprovalRequest } from "../types/domain";

const req = (tool: string, input: Record<string, unknown>): ApprovalRequest => ({
  id: "a",
  ts: 0,
  sessionDir: "/s",
  cwd: "/p",
  tool,
  input,
});

describe("perimeter (D14)", () => {
  it("matches exact hosts and dotted suffixes, like the Rust helper", () => {
    const allowed = [".github.com", "npmjs.org"];
    expect(hostAllowed("api.github.com", allowed)).toBe(true);
    expect(hostAllowed("github.com", allowed)).toBe(true);
    expect(hostAllowed("NPMJS.org", allowed)).toBe(true);
    expect(hostAllowed("registry.npmjs.org", allowed)).toBe(false);
    expect(hostAllowed("github.com.evil.example", allowed)).toBe(false);
  });

  it("finds the hosts a request would reach", () => {
    expect(
      hostsIn(
        req("Bash", {
          command: "curl -s https://api.github.com/repos | jq . && ssh deploy@prod.example.com ls",
        }),
      ),
    ).toEqual(["api.github.com", "prod.example.com"]);
    expect(hostsIn(req("WebFetch", { url: "https://docs.rs/serde" }))).toEqual(["docs.rs"]);
    expect(hostsIn(req("Bash", { command: "npm test" }))).toEqual([]);
    expect(hostsIn(req("Edit", { file_path: "https://x.y/z" }))).toEqual([]);
  });

  it("judges only when a perimeter is declared", () => {
    const fetch = req("WebFetch", { url: "https://pastebin.com/raw/abc" });
    expect(hostsOutside(fetch, undefined)).toEqual([]);
    expect(hostsOutside(fetch, [])).toEqual([]);
    expect(hostsOutside(fetch, [".github.com"])).toEqual(["pastebin.com"]);
    expect(
      hostsOutside(req("WebFetch", { url: "https://api.github.com/x" }), [".github.com"]),
    ).toEqual([]);
  });

  it("parses a typed host list", () => {
    expect(parseHosts(" github.com, .npmjs.org\n GitHub.com ")).toEqual([
      "github.com",
      ".npmjs.org",
    ]);
  });
});
