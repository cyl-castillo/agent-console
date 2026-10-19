// The shell, surface by surface: rail → list → centre → inspector over the
// fixture project. Every workbench tab has a story so each of the 19 panels
// stays on the visual and a11y contract.
import type { Meta, StoryObj } from "@storybook/react-vite";
import App from "../App";
import {
  approvalEdit,
  approvalFetch,
  approvalHigh,
  pendingApproval,
  ROOT,
} from "../dev/fixtures/defaults";
import { useBudgetStore } from "../stores/budgetStore";
import { useLiveStatusStore } from "../stores/liveStatusStore";
import { useDeclinesStore } from "../stores/declinesStore";
import { useReviewsStore } from "../stores/reviewsStore";
import { useSponsorStore } from "../stores/sponsorStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import type { FixtureMap } from "../dev/fixtures/registry";
import { useApprovalStore } from "../stores/approvalStore";
import { usePaletteStore } from "../stores/paletteStore";
import { useSessionStore } from "../stores/sessionStore";
import { useShellStore } from "../stores/shellStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";
import { useWorkbenchStore } from "../stores/workbenchStore";
import { WORKBENCH_TABS, type WorkbenchTab } from "../lib/workbenchTabs";
import { destinationForTab } from "../app/v2/destinations";
import type { Destination } from "../app/v2/destinations";
import { manyEvents, manyId, manySessions } from "../dev/fixtures/manySessions";
import { beginScenario, settle, sleep } from "./scenario";

interface Opts {
  fixtures?: FixtureMap;
  theme?: "dark" | "light";
  destination?: Destination;
  tab?: WorkbenchTab;
  centerTab?: "terminal" | "changes" | "preview";
  liveTerminal?: boolean;
  after?: () => Promise<void> | void;
}

function scenario(opts: Opts = {}): StoryObj {
  const { liveTerminal = true } = opts;
  return {
    loaders: [
      async () => {
        beginScenario(opts.fixtures, opts.theme);
        useSessionStore.getState().closeProject();
        useTerminalsStore.getState().clear();
        useUIStore.getState().setTab("terminal");
        useApprovalStore.setState({ queue: [] });
        usePaletteStore.getState().close();
        useToastStore.setState({ toasts: [] });
        if (opts.tab) localStorage.setItem(`agent-console:workbench-tab:${ROOT}`, opts.tab);
        await useSessionStore.getState().openProject(ROOT);
        return {};
      },
    ],
    play: async () => {
      await sleep(150);
      if (liveTerminal) {
        const st = useTerminalsStore.getState();
        const first = st.sessions[0];
        if (first) await st.resume(first.id);
      }
      if (opts.tab) useWorkbenchStore.getState().setTab(opts.tab);
      useShellStore.getState().goTo(opts.destination ?? "inbox");
      if (opts.centerTab) useUIStore.getState().setTab(opts.centerTab);
      await opts.after?.();
      await settle();
    },
  };
}

const meta: Meta = { title: "App v2", component: App, parameters: { layout: "fullscreen" } };
export default meta;

export const Inbox = scenario({
  fixtures: { approvals_pending: [pendingApproval] },
  after: () => useApprovalStore.setState({ queue: [pendingApproval] }),
});
export const InboxDecisions = scenario({
  fixtures: { approvals_pending: [pendingApproval, approvalHigh, approvalEdit, approvalFetch] },
  after: () => {
    useApprovalStore.setState({
      queue: [pendingApproval, approvalHigh, approvalEdit, approvalFetch],
    });
    const b = useBudgetStore.getState();
    b.setCap("sess-auth", 5);
    b.markBreached("sess-auth", 5.12);
    useLiveStatusStore
      .getState()
      .note({ ts: Date.now(), termId: "sess-auth", costUsd: 5.12, usedPct: 31 });
    useReviewsStore.getState().add(
      ROOT,
      {
        verdict: "block",
        findings: [
          {
            severity: "block",
            rule: "test_edited",
            title: "1 test file changed together with 1 source file",
            detail:
              "The prompt did not ask for test changes. A test edited alongside the code it covers is how an agent makes a failing change look green.",
            file: "tests/auth.test.ts",
            deterministic: true,
          },
          {
            severity: "medium",
            rule: "model",
            title: "Retry ceiling is hard-coded",
            detail:
              "src/retry.ts caps at 5 attempts with no configuration hook, unlike the summary claims.",
            file: "src/retry.ts",
            deterministic: false,
          },
        ],
        reviewer: "reviewer:codex",
        reviewerEngine: "codex",
        reviewerModel: null,
        producerModel: "claude-opus-5-5",
        termId: "sess-auth",
        turnId: "turn-001",
        caseId: "case-2026-10-13",
        files: 2,
        tokens: 812,
        costUsd: 0.01,
        rationale:
          "The assertion in tests/auth.test.ts went from toEqual to toBeTruthy, which would pass on the old bug too.",
        seq: 777,
      },
      Date.now() - 120_000,
    );
    useDeclinesStore.getState().add({
      seq: 99,
      ts: Date.now() - 60_000,
      caller: "claude-desktop",
      kind: "escalate",
      reason: "The fix needs the production database URL, which I will not guess.",
      suggestion: "Put it in the vault as DATABASE_URL, or run the migration yourself.",
      session: "sess-auth",
      project: ROOT,
    });
  },
});
export const InboxEmpty = scenario({
  fixtures: {
    sessions_list: [],
    jobs_board: { project: ROOT, settings: { parallelJobs: 1 }, cards: [], pending: [], busy: 0 },
  },
  liveTerminal: false,
});
/// First run after the wizard: the empty Inbox teaches with the checklist,
/// and a non-empty one keeps a single dismissable line (onboarding folded
/// into the shell, no banner above the top bar).
export const InboxOnboarding = scenario({
  fixtures: {
    sessions_list: [],
    jobs_board: { project: ROOT, settings: { parallelJobs: 1 }, cards: [], pending: [], busy: 0 },
  },
  liveTerminal: false,
  after: () => useOnboardingStore.setState({ bannerDismissed: false }),
});
export const InboxOnboardingHint = scenario({
  fixtures: { approvals_pending: [pendingApproval] },
  after: () => {
    useApprovalStore.setState({ queue: [pendingApproval] });
    useOnboardingStore.setState({ bannerDismissed: false });
  },
});
export const Sessions = scenario({ destination: "sessions" });
export const SessionsDrawerOpen = scenario({
  destination: "sessions",
  after: () => {
    if (!useShellStore.getState().drawerOpen) useShellStore.getState().toggleDrawer();
  },
});
/// The terminal as the whole stage: chat folded away.
export const SessionsTerminalOnly = scenario({
  destination: "sessions",
  after: () => {
    if (!useShellStore.getState().chatCollapsed) useShellStore.getState().toggleChat();
  },
});
export const SessionsNoTurns = scenario({
  destination: "sessions",
  fixtures: { testigo_list: [] },
});
export const SessionsChanges = scenario({ destination: "sessions", centerTab: "changes" });
/// Thirty sessions, eight live, three mid-turn: the list at the scale a week
/// reaches (CHAT-2026-10 F1 gate).
export const SessionsMany = scenario({
  destination: "sessions",
  fixtures: { sessions_list: manySessions, testigo_list: manyEvents },
  after: async () => {
    const st = useTerminalsStore.getState();
    for (let k = 2; k <= 8; k++) await st.resume(manyId(k));
    st.setActive(manyId(1));
  },
});
export const WorkJobs = scenario({ destination: "work", tab: "jobs" });
export const WorkRoom = scenario({ destination: "work", tab: "roundtable" });
/// D11: a sponsor that decides low-risk requests alone, with a weekly budget.
export const WorkJobsSponsor = scenario({
  destination: "work",
  tab: "jobs",
  fixtures: {
    sponsor_get: {
      objective: "Ship billing v2 without regressions",
      owner: "Carlos",
      weeklyBudgetUsd: 40,
      riskThreshold: "low",
      autoApprove: true,
    },
  },
  after: () => {
    useLiveStatusStore.getState().note({ ts: Date.now(), termId: "sess-auth", costUsd: 12.4 });
    useSponsorStore.setState({ autoApproved: 7 });
  },
});
export const ProofLedger = scenario({ destination: "proof", tab: "proof" });
export const ProofTurnSelected = scenario({
  destination: "proof",
  tab: "proof",
  after: () => useShellStore.getState().selectTurn("turn-001"),
});
export const SettingsPermissions = scenario({ destination: "settings", tab: "permissions" });
export const SettingsSkills = scenario({ destination: "settings", tab: "skills" });
export const SettingsMcp = scenario({ destination: "settings", tab: "mcp" });
export const LightInbox = scenario({ theme: "light" });
export const LightSessions = scenario({ theme: "light", destination: "sessions" });

// ---- Every panel ------------------------------------------------------------

const covered = new Set<WorkbenchTab>([
  "jobs",
  "roundtable",
  "proof",
  "permissions",
  "skills",
  "mcp",
]);
const panel: Partial<Record<WorkbenchTab, StoryObj>> = {};
for (const tab of WORKBENCH_TABS) {
  if (!covered.has(tab)) panel[tab] = scenario({ destination: destinationForTab(tab), tab });
}
export const PanelTurns = panel.turns!;
export const PanelAdvisor = panel.advisor!;
export const PanelLearning = panel.learning!;
export const PanelContext = panel.context!;
export const PanelSchedule = panel.schedule!;
export const PanelVault = panel.vault!;
export const PanelPlugins = panel.plugins!;
export const PanelJira = panel.jira!;
export const PanelTeams = panel.teams!;
export const PanelAgenda = panel.agenda!;
export const PanelNotes = panel.notes!;
export const PanelTransfer = panel.transfer!;
export const PanelFeedback = panel.feedback!;
