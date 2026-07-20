import { describe, expect, it, vi } from "vitest";

// The store reads localStorage at module-init, so the stub must exist before
// the import — hence hoisted.
const storage = vi.hoisted(() => {
  const map = new Map<string, string>();
  const stub = {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      map.set(k, v);
    },
    removeItem: (k: string) => {
      map.delete(k);
    },
    clear: () => map.clear(),
  };
  vi.stubGlobal("localStorage", stub);
  return { map };
});

const KEY = "agent-console.onboarding.v1";

import { useOnboardingStore } from "./onboardingStore";

describe("onboarding progress", () => {
  it("starts with everything unseen", () => {
    useOnboardingStore.getState().reset();
    const s = useOnboardingStore.getState();
    expect(s.seenWelcome).toBe(false);
    expect(s.visitedProof).toBe(false);
    expect(s.bannerDismissed).toBe(false);
  });

  it("marks persist to localStorage so progress survives restarts", () => {
    useOnboardingStore.getState().reset();
    useOnboardingStore.getState().markVisitedProof();
    useOnboardingStore.getState().markPromptedClaude();

    expect(useOnboardingStore.getState().visitedProof).toBe(true);
    const stored = JSON.parse(storage.map.get(KEY)!);
    expect(stored.visitedProof).toBe(true);
    expect(stored.promptedClaude).toBe(true);
    // Untouched flags persist as false, not dropped.
    expect(stored.createdSkill).toBe(false);
  });

  it("reset clears both memory and storage", () => {
    useOnboardingStore.getState().markSeenWelcome();
    useOnboardingStore.getState().reset();
    expect(useOnboardingStore.getState().seenWelcome).toBe(false);
    expect(JSON.parse(storage.map.get(KEY)!).seenWelcome).toBe(false);
  });

  it("a partial or corrupt stored blob falls back to defaults per-field", async () => {
    // Partial: unknown-future-field plus one known flag. Missing keys default.
    storage.map.set(KEY, JSON.stringify({ visitedPermissions: true, futureFlag: 1 }));
    vi.resetModules();
    const fresh = await import("./onboardingStore");
    let s = fresh.useOnboardingStore.getState();
    expect(s.visitedPermissions).toBe(true);
    expect(s.seenWelcome).toBe(false);

    // Corrupt JSON: everything defaults, no throw at import time.
    storage.map.set(KEY, "{not json");
    vi.resetModules();
    const fresh2 = await import("./onboardingStore");
    s = fresh2.useOnboardingStore.getState();
    expect(s.visitedPermissions).toBe(false);
  });
});
