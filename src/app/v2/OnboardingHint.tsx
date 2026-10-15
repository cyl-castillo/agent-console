import { X } from "lucide-react";
import { useOverlaysStore } from "../../stores/overlaysStore";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { useOnboardingSteps } from "../useOnboardingSteps";

/// Onboarding folded into the shell (UI-2026-10.md F2): one dismissable line
/// in the Inbox while getting-started steps remain, opening the same guide
/// the v1 banner did. The empty Inbox shows the checklist itself.
export function OnboardingHint() {
  const { pending, dismissed, dismiss } = useOnboardingSteps();
  const show = useOverlaysStore((s) => s.show);
  if (dismissed || pending === 0) return null;
  return (
    <div
      className="flex items-center gap-2 border-b border-border bg-[var(--accent-12)] px-4 py-1.5 font-sans text-small text-foreground"
      data-testid="onboarding-hint"
      role="status"
    >
      <span>
        {pending} getting-started step{pending === 1 ? "" : "s"} left.
      </span>
      <Button size="sm" variant="ghost" onClick={() => show("gettingStarted")}>
        Open guide
      </Button>
      <span className="ml-auto" />
      <IconButton label="Don't show again" size="sm" icon={<X />} onClick={dismiss} />
    </div>
  );
}
