import type { OnboardingState } from "../stores/onboardingStore";

export interface OnboardingStep {
  label: string;
  done: boolean;
}

/// The getting-started checklist, derived from persisted one-shots and live
/// counts (a prompt sent, an advisor run, a skill installed) — the same
/// arithmetic the v1 banner used, shared so both shells agree on "N left".
export function onboardingSteps(
  ob: Pick<
    OnboardingState,
    "promptedClaude" | "triggeredAdvisor" | "createdSkill" | "visitedPermissions"
  >,
  live: {
    hasProject: boolean;
    recentPrompts: number;
    advisorItems: number;
    installedSkills: number;
  },
): OnboardingStep[] {
  return [
    { label: "Open a project", done: live.hasProject },
    { label: "Send the agent a prompt", done: ob.promptedClaude || live.recentPrompts > 0 },
    { label: "Run the advisor once", done: ob.triggeredAdvisor || live.advisorItems > 0 },
    { label: "Create a skill", done: ob.createdSkill || live.installedSkills > 0 },
    { label: "Review permissions", done: ob.visitedPermissions },
  ];
}

export const pendingCount = (steps: OnboardingStep[]) => steps.filter((s) => !s.done).length;
