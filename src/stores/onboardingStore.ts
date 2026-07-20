import { create } from "zustand";

const STORAGE_KEY = "agent-console.onboarding.v1";

/// Persisted facts about the user's progress through the getting-started
/// checklist. Anything that can be derived from live store state is NOT
/// stored here — only events ("I clicked X", "I dismissed Y") and one-shots.
export interface OnboardingState {
  /// True once the modal has auto-opened the first time. Stops it from
  /// reappearing on every launch.
  seenWelcome: boolean;
  /// True once the user has visited the Permissions tab at least once.
  visitedPermissions: boolean;
  /// True once the user has run an Advisor analysis at least once.
  triggeredAdvisor: boolean;
  /// True once the user has created at least one skill via the Advisor.
  createdSkill: boolean;
  /// True once the user has sent at least one prompt to Claude in any terminal.
  promptedClaude: boolean;
  /// True once the user has opened the Proof tab from the guide ("what makes
  /// you stay" section).
  visitedProof: boolean;
  /// User dismissed the unobtrusive progress banner.
  bannerDismissed: boolean;
}

const DEFAULT: OnboardingState = {
  seenWelcome: false,
  visitedPermissions: false,
  triggeredAdvisor: false,
  createdSkill: false,
  promptedClaude: false,
  visitedProof: false,
  bannerDismissed: false,
};

function load(): OnboardingState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    const parsed = JSON.parse(raw);
    return { ...DEFAULT, ...parsed };
  } catch {
    return DEFAULT;
  }
}

function persist(s: OnboardingState) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(s));
  } catch {
    /* ignore */
  }
}

interface Store extends OnboardingState {
  markSeenWelcome: () => void;
  markVisitedPermissions: () => void;
  markTriggeredAdvisor: () => void;
  markCreatedSkill: () => void;
  markPromptedClaude: () => void;
  markVisitedProof: () => void;
  dismissBanner: () => void;
  reset: () => void;
}

export const useOnboardingStore = create<Store>((set, get) => {
  function patch(p: Partial<OnboardingState>) {
    const next = { ...get(), ...p };
    persist({
      seenWelcome: next.seenWelcome,
      visitedPermissions: next.visitedPermissions,
      triggeredAdvisor: next.triggeredAdvisor,
      createdSkill: next.createdSkill,
      promptedClaude: next.promptedClaude,
      visitedProof: next.visitedProof,
      bannerDismissed: next.bannerDismissed,
    });
    set(p);
  }
  return {
    ...load(),
    markSeenWelcome: () => patch({ seenWelcome: true }),
    markVisitedPermissions: () => patch({ visitedPermissions: true }),
    markTriggeredAdvisor: () => patch({ triggeredAdvisor: true }),
    markCreatedSkill: () => patch({ createdSkill: true }),
    markPromptedClaude: () => patch({ promptedClaude: true }),
    markVisitedProof: () => patch({ visitedProof: true }),
    dismissBanner: () => patch({ bannerDismissed: true }),
    reset: () => {
      persist(DEFAULT);
      set(DEFAULT);
    },
  };
});
