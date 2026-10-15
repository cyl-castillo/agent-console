import { useOnboardingSteps } from "../app/useOnboardingSteps";

interface Props {
  onOpen: () => void;
}

/// Slim banner shown above the topbar while the user still has pending
/// getting-started items, unless they explicitly dismissed it.
export function OnboardingBanner({ onOpen }: Props) {
  const { pending, dismissed, dismiss } = useOnboardingSteps();
  if (dismissed || pending === 0) return null;

  return (
    <div className="onboarding-banner">
      <span className="ob-dot" />
      <span className="ob-text">
        {pending} getting-started step{pending === 1 ? "" : "s"} left.
      </span>
      <button className="wb-link" onClick={onOpen}>
        Open guide
      </button>
      <button className="ob-dismiss" onClick={dismiss} title="Don't show again">
        ×
      </button>
    </div>
  );
}
