import { describe, expect, it } from "vitest";

import { firstSentences, turnAnnouncement } from "./announce";

describe("firstSentences", () => {
  it("cuts on sentence ends only", () => {
    const t = "Listo. Cambié 3 archivos (v1.2). Los tests pasaron! Queda el deploy?";
    expect(firstSentences(t, 1)).toBe("Listo.");
    expect(firstSentences(t, 2)).toBe("Listo. Cambié 3 archivos (v1.2).");
    expect(firstSentences(t, 10)).toBe(t);
    expect(firstSentences("sin punto final", 1)).toBe("sin punto final");
  });
});

describe("turnAnnouncement", () => {
  it("speaks two sentences of the summary, naming the session when it's not on screen", () => {
    const s = "Arreglé el login. Los tests pasan. Falta el deploy.";
    expect(turnAnnouncement(s, "fixy", true, true)).toBe("Arreglé el login. Los tests pasan.");
    expect(turnAnnouncement(s, "fixy", false, true)).toBe(
      "fixy: Arreglé el login. Los tests pasan.",
    );
  });
  it("falls back to a plain 'turn finished' without a summary", () => {
    expect(turnAnnouncement(undefined, "fixy", true, true)).toBe("Turno terminado.");
    expect(turnAnnouncement("  ", undefined, false, false)).toBe("Turn finished.");
  });
});
