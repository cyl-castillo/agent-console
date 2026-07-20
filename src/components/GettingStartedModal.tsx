import { useEffect, useMemo } from "react";

import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { useSessionStore } from "../stores/sessionStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useAdvisorStore } from "../stores/advisorStore";
import { useSkillsStore } from "../stores/skillsStore";
import { useSchedulerStore } from "../stores/schedulerStore";
import { usePreflightStore, toolStatus } from "../stores/preflightStore";
import type { WorkbenchTab } from "../lib/workbenchTabs";

interface Props {
  onClose: () => void;
  /// Called when a checklist item asks the user to switch to a workbench tab.
  /// Any tab: the guide's second section points at the app's differentiators
  /// (Proof, Rooms, Schedule) — the old narrow type couldn't reach them.
  onJumpToTab: (tab: WorkbenchTab) => void;
}

interface Step {
  key: string;
  title: string;
  description: string;
  done: boolean;
  action?: { label: string; onClick: () => void };
}

export function GettingStartedModal({ onClose, onJumpToTab }: Props) {
  const project = useSessionStore((s) => s.project);
  const onboarding = useOnboardingStore();
  const advisorItems = useAdvisorStore((s) => s.items);
  const installedSkills = useSkillsStore((s) => s.installed);
  const recentPromptsCount = useSkillsStore((s) => s.recent.length);
  const scheduledJobs = useSchedulerStore((s) => s.jobs.length);
  const preflight = usePreflightStore((s) => s.result);
  const checkPreflight = usePreflightStore((s) => s.check);

  // Mark seen once when opened so it doesn't auto-open again on next launch.
  useEffect(() => {
    if (!onboarding.seenWelcome) onboarding.markSeenWelcome();
    void checkPreflight();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const requirementsReady =
    !!toolStatus(preflight, "claude")?.found && !!toolStatus(preflight, "node")?.found;

  const jump = (tab: WorkbenchTab) => () => {
    onJumpToTab(tab);
    onClose();
  };

  const firstSteps: Step[] = useMemo(
    () => [
      {
        key: "requirements",
        title: "Install the requirements",
        description:
          "You need the Claude CLI (`npm i -g @anthropic-ai/claude-code`, then run `claude` once to log in) and Node ≥ 20.",
        done: requirementsReady,
      },
      {
        key: "open-project",
        title: "Open a project",
        description: "Every terminal starts in the active project's directory.",
        done: !!project,
      },
      {
        key: "talk-to-claude",
        title: "Talk to Claude in the terminal",
        description: "New terminals auto-run `claude`. Type a question and hit enter.",
        done: onboarding.promptedClaude || recentPromptsCount > 0,
      },
      {
        key: "permissions",
        title: "Review your permissions",
        description: "See which tools Claude can use without asking, and tune to taste.",
        done: onboarding.visitedPermissions,
        action: { label: "Open Permissions →", onClick: jump("permissions") },
      },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [project, onboarding, recentPromptsCount, requirementsReady],
  );

  const staySteps: Step[] = useMemo(
    () => [
      {
        key: "proof",
        title: "See your work witnessed (Proof)",
        description:
          "Every prompt, approval and result lands in a local evidence ledger. Export a signed proof packet anyone can verify in a browser — attach it to a PR.",
        done: onboarding.visitedProof,
        action: {
          label: "Open Proof →",
          onClick: () => {
            onboarding.markVisitedProof();
            onJumpToTab("proof");
            onClose();
          },
        },
      },
      {
        key: "advisor",
        title: "Generate skills with the Advisor",
        description: "The Advisor analyzes your project and proposes concrete skills.",
        done: onboarding.triggeredAdvisor || advisorItems.length > 0,
        action: { label: "Open Advisor →", onClick: jump("advisor") },
      },
      {
        key: "create-skill",
        title: "Create your first skill",
        description: "From the Advisor's list, review a recommendation and hit Create.",
        done: onboarding.createdSkill || installedSkills.length > 0,
        action: { label: "See skills →", onClick: jump("skills") },
      },
      {
        key: "schedule",
        title: "Put an agent on a schedule",
        description:
          "Suggest-only jobs on a clock — a nightly review, a weekly dependency check. Nothing mutates without you.",
        done: scheduledJobs > 0,
        action: { label: "Open Schedule →", onClick: jump("schedule") },
      },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [onboarding, advisorItems.length, installedSkills.length, scheduledJobs],
  );

  const steps = [...firstSteps, ...staySteps];
  const completed = steps.filter((s) => s.done).length;

  const renderSteps = (list: Step[]) => (
    <ul className="gs-steps">
      {list.map((step) => (
        <li key={step.key} className={`gs-step ${step.done ? "done" : ""}`}>
          <span className="gs-check" aria-hidden>
            {step.done ? "✓" : "○"}
          </span>
          <div className="gs-step-body">
            <div className="gs-step-title">{step.title}</div>
            <div className="gs-step-desc">{step.description}</div>
          </div>
          {step.action && !step.done && (
            <button className="wb-cta wb-cta-sm" onClick={step.action.onClick}>
              {step.action.label}
            </button>
          )}
        </li>
      ))}
    </ul>
  );

  return (
    <Modal onClose={onClose} className="getting-started-modal" ariaLabel="Getting Started">
      <div className="gs-head">
        <div>
          <div className="gs-title">Getting Started</div>
          <div className="gs-subtitle">
            {completed} of {steps.length} done
          </div>
        </div>
        <button className="gs-close" onClick={onClose} title="Close" aria-label="Close">
          <Icon name="x" size={14} />
        </button>
      </div>

      <div className="gs-progress">
        <div className="gs-progress-fill" style={{ width: `${(completed / steps.length) * 100}%` }} />
      </div>

      <div className="gs-section-label">The first ten minutes</div>
      {renderSteps(firstSteps)}
      <div className="gs-section-label">What makes it yours</div>
      {renderSteps(staySteps)}

      <div className="gs-foot">
        <p className="gs-hint">
          This guide lives under the <strong>?</strong> icon in the topbar. Come back anytime.
        </p>
        <button onClick={onClose}>Close</button>
      </div>
    </Modal>
  );
}
