import { useMemo } from "react";
import { useAdvisorStore } from "../stores/advisorStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSkillsStore } from "../stores/skillsStore";
import { onboardingSteps, pendingCount, type OnboardingStep } from "./onboardingSteps";

/// Live getting-started checklist for the open project, plus whether the
/// user asked not to be reminded.
export function useOnboardingSteps(): {
  steps: OnboardingStep[];
  pending: number;
  dismissed: boolean;
  dismiss: () => void;
} {
  const hasProject = useSessionStore((s) => s.project !== null);
  const ob = useOnboardingStore();
  const advisorItems = useAdvisorStore((s) => s.items.length);
  const installedSkills = useSkillsStore((s) => s.installed.length);
  const recentPrompts = useSkillsStore((s) => s.recent.length);
  const steps = useMemo(
    () => onboardingSteps(ob, { hasProject, recentPrompts, advisorItems, installedSkills }),
    [ob, hasProject, recentPrompts, advisorItems, installedSkills],
  );
  return {
    steps,
    pending: pendingCount(steps),
    dismissed: ob.bannerDismissed,
    dismiss: ob.dismissBanner,
  };
}
