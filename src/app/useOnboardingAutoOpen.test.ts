import { describe, expect, it, vi } from "vitest";

vi.mock("../ipc/tauri", () => ({ ipc: {} }));

import { autoOpenFor } from "./useOnboardingAutoOpen";

const fresh = { wizardDone: false, seenWelcome: false, engines: 0, wizardBusy: false };

describe("what opens by itself on a first run", () => {
  it("waits for the setup check, then picks the wizard or the guide", () => {
    expect(autoOpenFor({ ...fresh, engines: null })).toBeNull();
    expect(autoOpenFor(fresh)).toBe("wizard");
    expect(autoOpenFor({ ...fresh, engines: 2 })).toBe("gettingStarted");
    expect(autoOpenFor({ ...fresh, engines: 2, seenWelcome: true })).toBeNull();
  });

  it("opens nothing while the wizard is open or stepped aside", () => {
    // Stepped aside for an install: a re-check that still finds no engine
    // must not put the wizard back over the terminal.
    expect(autoOpenFor({ ...fresh, wizardBusy: true })).toBeNull();
    // Back in the wizard, the engine it just installed shows up: the guide
    // must not open on top of the wizard's login step.
    expect(autoOpenFor({ ...fresh, engines: 1, wizardBusy: true })).toBeNull();
  });

  it("opens the guide once the wizard is finished, if it was never seen", () => {
    expect(autoOpenFor({ ...fresh, wizardDone: true, engines: 1 })).toBe("gettingStarted");
    expect(autoOpenFor({ ...fresh, wizardDone: true, engines: null })).toBe("gettingStarted");
    expect(autoOpenFor({ ...fresh, wizardDone: true, seenWelcome: true })).toBeNull();
  });
});
