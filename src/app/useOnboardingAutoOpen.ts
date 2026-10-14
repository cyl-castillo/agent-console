import { useEffect } from "react";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useOverlaysStore } from "../stores/overlaysStore";
import { usePreflightStore, toolStatus } from "../stores/preflightStore";

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
    const noEngine =
      !toolStatus(preflight, "claude")?.found && !toolStatus(preflight, "codex")?.found;
    if (noEngine) show("wizard");
    else if (!seenWelcome) show("gettingStarted");
  }, [seenWelcome, wizardDone, preflight, show]);
}
