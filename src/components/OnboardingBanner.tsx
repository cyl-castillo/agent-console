import { useMemo } from "react";

import { useSessionStore } from "../stores/sessionStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useAdvisorStore } from "../stores/advisorStore";
import { useSkillsStore } from "../stores/skillsStore";

interface Props {
  onOpen: () => void;
}

/// Slim banner shown above the topbar while the user still has pending
/// getting-started items, unless they explicitly dismissed it.
export function OnboardingBanner({ onOpen }: Props) {
  const project = useSessionStore((s) => s.project);
  const ob = useOnboardingStore();
  const advisorItems = useAdvisorStore((s) => s.items.length);
  const installedSkills = useSkillsStore((s) => s.installed.length);
  const recentPrompts = useSkillsStore((s) => s.recent.length);

  const pending = useMemo(() => {
    let n = 0;
    if (!project) n++;
    if (!(ob.promptedClaude || recentPrompts > 0)) n++;
    if (!(ob.triggeredAdvisor || advisorItems > 0)) n++;
    if (!(ob.createdSkill || installedSkills > 0)) n++;
    if (!ob.visitedPermissions) n++;
    return n;
  }, [project, ob, advisorItems, installedSkills, recentPrompts]);

  if (ob.bannerDismissed || pending === 0) return null;

  return (
    <div className="onboarding-banner">
      <span className="ob-dot" />
      <span className="ob-text">
        {pending} getting-started step{pending === 1 ? "" : "s"} left.
      </span>
      <button className="wb-link" onClick={onOpen}>Open guide</button>
      <button className="ob-dismiss" onClick={ob.dismissBanner} title="Don't show again">×</button>
    </div>
  );
}
