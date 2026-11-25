import { useEffect } from "react";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useOverlaysStore } from "../stores/overlaysStore";
import { usePreflightStore, enginesFound } from "../stores/preflightStore";
import { isSetupAside } from "../lib/setupAside";

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
    if (wizardDone) {
      if (!seenWelcome) show("gettingStarted");
      return;
    }
    if (!preflight) return;
    if (enginesFound(preflight).length === 0) {
      // Stepped aside for an install or a login: a re-check that still finds
      // no engine must not put the wizard back over the terminal.
      if (!isSetupAside()) show("wizard");
    } else if (!seenWelcome) show("gettingStarted");
  }, [seenWelcome, wizardDone, preflight, show]);
}
