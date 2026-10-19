// Story plumbing: reset every store-independent piece of browser state, apply
// a fixture scenario and give Playwright a readiness signal once the app has
// settled (stores hydrate asynchronously after mount).
import { applyFixtures, type FixtureMap } from "../dev/fixtures/registry";
import { emit, resetEventBus } from "../dev/tauri-shim/event";
import { getFixture, invokeLog, setFixture } from "../dev/fixtures/registry";
import { useThemeStore } from "../stores/themeStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { toastManager } from "../ui/Toast";
import { installAgentBridge } from "../lib/agentBridge";
import { useDeclinesStore } from "../stores/declinesStore";
import { useReviewsStore } from "../stores/reviewsStore";
import { useSponsorStore } from "../stores/sponsorStore";
import { useTermSignalsStore } from "../stores/termSignalsStore";

export const READY_ATTR = "data-story-ready";

// Fresh sessions, rooms and notes mint ids with crypto.randomUUID(); make the
// sequence deterministic so screenshots that show an id do not flake.
let uuidSeq = 0;
function seedUuids() {
  uuidSeq = 0;
  Object.defineProperty(crypto, "randomUUID", {
    configurable: true,
    value: () => `00000000-0000-4000-8000-${String(++uuidSeq).padStart(12, "0")}`,
  });
}

export function beginScenario(fixtures: FixtureMap = {}, theme: "dark" | "light" = "dark") {
  document.body.removeAttribute(READY_ATTR);
  seedUuids();
  try {
    localStorage.clear();
  } catch {
    /* ignore */
  }
  resetEventBus();
  toastManager.close();
  useDeclinesStore.setState({ items: [] });
  useReviewsStore.setState({ items: [] });
  useSponsorStore.setState({ project: null, sponsor: null, autoApproved: 0 });
  useTermSignalsStore.setState({ bySession: {}, tails: {} });
  installAgentBridge();
  // Specs drive the backend side: push events the Rust side would emit and
  // read which commands the webview invoked in return.
  (window as unknown as { __fixtures: unknown }).__fixtures = {
    emit,
    invokeLog,
    setFixture,
    getFixture,
  };
  applyFixtures(fixtures);
  useThemeStore.getState().setTheme(theme);
  // Stories render the working app, not first-run flows, unless they opt in.
  useOnboardingStore.setState({ seenWelcome: true, wizardDone: true, bannerDismissed: true });
}

export const sleep = (ms: number) => new Promise<void>((r) => window.setTimeout(r, ms));

export async function settle(ms = 400): Promise<void> {
  await sleep(ms);
  document.body.setAttribute(READY_ATTR, "1");
}
