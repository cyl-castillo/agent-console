import { describe, expect, it } from "vitest";

import { checkGlyph, checkNote, checkTitle } from "./checkStatus";

describe("what a check line says about its exit code", () => {
  it("says whose exit code it is when the line's was another command's", () => {
    const piped = { status: "unknown", exitCode: 0, exitCodeOf: "line" };
    expect(checkNote(piped)).toBe(" · exit 0 is the line's, not the runner's");
    expect(checkNote({ status: "unknown", exitCode: 1, exitCodeOf: "line" })).toBe(
      " · exit 1 is the line's, not the runner's",
    );
    expect(checkTitle("unknown", "line")).toContain("another command");
    expect(checkTitle("unknown", "line")).not.toContain("none was reported");
  });

  it("says the exit code is unknown only when none was seen", () => {
    expect(checkNote({ status: "unknown" })).toBe(" · exit unknown");
    expect(checkTitle("unknown")).toContain("no exit code was reported");
    // A ledger written before `exitCodeOf` existed: unknown meant no code.
    expect(checkNote({ status: "unknown", exitCode: undefined, exitCodeOf: undefined })).toBe(
      " · exit unknown",
    );
  });

  it("keeps the other verdicts as they were", () => {
    expect(checkNote({ status: "failed", exitCode: 101, exitCodeOf: "runner" })).toBe(" exit 101");
    expect(checkNote({ status: "failed" })).toBe("");
    expect(checkNote({ status: "empty", exitCode: 0 })).toBe(" · ran 0 tests");
    expect(checkNote({ status: "passed", exitCode: 0, exitCodeOf: "runner" })).toBe("");
    expect(checkNote({ status: "interrupted" })).toBe("");
  });

  it("never draws an unknown check with the interrupted glyph", () => {
    expect(checkGlyph("unknown")).toBe("?");
    expect(checkGlyph("interrupted")).not.toBe(checkGlyph("unknown"));
  });
});
