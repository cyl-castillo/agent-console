import { describe, expect, it } from "vitest";
import { OSC_7501_REPLY, parseProgramStatus } from "./programStatus";

const b64 = (s: string) => btoa(unescape(encodeURIComponent(s)));

describe("OSC 7501 program status (F5)", () => {
  it("parses Claude Code's emitter shape, decoding the base64 texts", () => {
    expect(parseProgramStatus("state=working:app=claude-code")).toEqual({
      state: "working",
      app: "claude-code",
    });
    expect(
      parseProgramStatus(
        `state=blocked:app=claude-code:kind=permission:progress=40:title=${b64("Allow Bash?")}:msg=${b64("npm test")}`,
      ),
    ).toEqual({
      state: "blocked",
      app: "claude-code",
      kind: "permission",
      progress: 40,
      title: "Allow Bash?",
      msg: "npm test",
    });
    expect(parseProgramStatus("state=clear:id=build/test")).toEqual({
      state: "clear",
      id: "build/test",
    });
    expect(parseProgramStatus("state=done:app=claude-code:progress=150")?.progress).toBe(100);
  });

  it("treats the probe and junk as not-a-status, and answers the probe with an OSC 7501 starting with ?", () => {
    expect(parseProgramStatus("?")).toBeNull();
    expect(parseProgramStatus("state=weird")).toBeNull();
    expect(parseProgramStatus("app=claude-code")).toBeNull();
    expect(parseProgramStatus("")).toBeNull();
    expect(OSC_7501_REPLY).toBe("\x1b]7501;?\x1b\\");
  });
});
