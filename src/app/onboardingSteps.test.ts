import { describe, expect, it } from "vitest";
import { onboardingSteps, pendingCount } from "./onboardingSteps";

const ob = {
  promptedClaude: false,
  triggeredAdvisor: false,
  createdSkill: false,
  visitedPermissions: false,
};
const live = { hasProject: true, recentPrompts: 0, advisorItems: 0, installedSkills: 0 };

describe("onboardingSteps", () => {
  it("counts the one-shots and the live evidence alike", () => {
    expect(pendingCount(onboardingSteps(ob, live))).toBe(4);
    expect(pendingCount(onboardingSteps(ob, { ...live, hasProject: false }))).toBe(5);
    // A prompt sent, a skill installed and an advisor run count even when
    // the one-shots were never written (older installs, other machines).
    expect(
      pendingCount(
        onboardingSteps(ob, { ...live, recentPrompts: 1, installedSkills: 2, advisorItems: 1 }),
      ),
    ).toBe(1);
    expect(
      pendingCount(
        onboardingSteps(
          {
            promptedClaude: true,
            triggeredAdvisor: true,
            createdSkill: true,
            visitedPermissions: true,
          },
          live,
        ),
      ),
    ).toBe(0);
  });

  it("keeps a stable order with the project first", () => {
    expect(onboardingSteps(ob, live).map((s) => s.label)).toEqual([
      "Open a project",
      "Send the agent a prompt",
      "Run the advisor once",
      "Create a skill",
      "Review permissions",
    ]);
  });
});
