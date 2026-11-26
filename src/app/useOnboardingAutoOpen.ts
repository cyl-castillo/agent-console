import { useEffect } from "react";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useOverlaysStore } from "../stores/overlaysStore";
import { usePreflightStore, enginesFound } from "../stores/preflightStore";
import { isSetupAside } from "../lib/setupAside";

/// What to open by itself, if anything. `engines` is null until the
/// preflight probe has answered. `wizardBusy` is the wizard open, or stepped
/// aside for a terminal: it owns the screen until it is finished, so a
/// re-check must neither put it back over the terminal nor open the guide on
/// top of it (two modals, each trapping focus) when the engine it just
/// installed shows up.
export function autoOpenFor(s: {
  wizardDone: boolean;
  seenWelcome: boolean;
  engines: number | null;
  wizardBusy: boolean;
}): "wizard" | "gettingStarted" | null {
  if (s.wizardDone) return s.seenWelcome ? null : "gettingStarted";
  if (s.engines === null || s.wizardBusy) return null;
  if (s.engines === 0) return "wizard";
  return s.seenWelcome ? null : "gettingStarted";
}

/// First-time auto-open. If NO agent CLI is installed this machine belongs
/// to a first-run novice: open the wizard (engine → install → login) instead
/// of the checklist. The decision waits for the preflight probe — never flash
/// the wizard on a machine that turns out to be fully set up.
export function useOnboardingAutoOpen(): void {
  const wizardDone = useOnboardingStore((s) => s.wizardDone);
  const seenWelcome = useOnboardingStore((s) => s.seenWelcome);
  const preflight = usePreflightStore((s) => s.result);
  const checkPreflight = usePreflightStore((s) => s.check);
  const show = useOverlaysStore((s) => s.show);

  useEffect(() => {
    void checkPreflight();
  }, [checkPreflight]);
  useEffect(() => {
    const overlay = autoOpenFor({
      wizardDone,
      seenWelcome,
      engines: preflight ? enginesFound(preflight).length : null,
      // Read when the answer lands, not subscribed to: the wizard closing
      // without finishing is not a reason to open anything.
      wizardBusy: useOverlaysStore.getState().open.wizard || isSetupAside(),
    });
    if (overlay) show(overlay);
  }, [seenWelcome, wizardDone, preflight, show]);
}
