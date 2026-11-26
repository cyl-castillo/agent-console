import { beforeEach, describe, expect, it, vi } from "vitest";

const show = vi.fn();
const dismiss = vi.fn();
vi.mock("../stores/toastStore", () => ({
  useToastStore: { getState: () => ({ show, dismiss }) },
}));

import {
  asideEngine,
  backToSetup,
  isSetupAside,
  setupResumed,
  setupStepsAside,
} from "./setupAside";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useOverlaysStore } from "../stores/overlaysStore";

const wizardOpen = () => useOverlaysStore.getState().open.wizard;

describe("the setup wizard steps aside for a terminal", () => {
  beforeEach(() => {
    setupResumed();
    show.mockReset();
    dismiss.mockReset();
    let n = 0;
    show.mockImplementation(() => `toast-${++n}`);
    useOverlaysStore.getState().show("wizard");
  });

  it("hides the wizard without finishing it, and leaves a way back that stays", () => {
    setupStepsAside("codex");

    expect(wizardOpen()).toBe(false);
    // Closing the wizard marks it done for good; stepping aside must not.
    expect(useOnboardingStore.getState().wizardDone).toBe(false);
    expect(isSetupAside()).toBe(true);
    expect(asideEngine()).toBe("codex");

    expect(show).toHaveBeenCalledTimes(1);
    const [, tone, opts] = show.mock.calls[0];
    expect(tone).toBe("info");
    // 0 = until dismissed: an install can take longer than any timeout.
    expect(opts.timeoutMs).toBe(0);
    expect(opts.action.label).toBe("Back to setup");

    opts.action.onClick();
    expect(wizardOpen()).toBe(true);
  });

  it("is over once the wizard is open again: the toast goes, the engine is forgotten", () => {
    setupStepsAside("claude");
    backToSetup();
    // What the wizard does when it mounts.
    expect(asideEngine()).toBe("claude");
    setupResumed();

    expect(dismiss).toHaveBeenCalledWith("toast-1");
    expect(isSetupAside()).toBe(false);
    expect(asideEngine()).toBeNull();
    // Nothing to resume: a second call is a no-op.
    setupResumed();
    expect(dismiss).toHaveBeenCalledTimes(1);
  });

  it("keeps one way back, not one per terminal opened", () => {
    setupStepsAside("claude");
    useOverlaysStore.getState().show("wizard");
    setupStepsAside("claude");

    expect(dismiss).toHaveBeenCalledWith("toast-1");
    expect(show).toHaveBeenCalledTimes(2);
    expect(isSetupAside()).toBe(true);
  });
});
