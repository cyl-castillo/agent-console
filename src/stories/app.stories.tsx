// The whole app, surface by surface, rendered in a browser against fixture
// data. These stories are the visual contract: Playwright screenshots every
// one of them (tests/visual) and the a11y addon audits each.
import type { Meta, StoryObj } from "@storybook/react-vite";
import App from "../App";
import { pendingApproval, ROOT } from "../dev/fixtures/defaults";
import { useApprovalStore } from "../stores/approvalStore";
import { useConfirmStore } from "../stores/confirmStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { usePaletteStore } from "../stores/paletteStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";
import { WORKBENCH_TABS, type WorkbenchTab } from "../lib/workbenchTabs";
import { beginScenario, settle, sleep } from "./scenario";
import type { FixtureMap } from "../dev/fixtures/registry";

interface Opts {
  fixtures?: FixtureMap;
  theme?: "dark" | "light";
  project?: boolean;
  workbenchTab?: WorkbenchTab;
  centerTab?: "terminal" | "changes" | "preview";
  /// Resume the first persisted session so a live terminal is mounted.
  liveTerminal?: boolean;
  after?: () => Promise<void> | void;
}

function resetStores() {
  useSessionStore.getState().closeProject();
  useTerminalsStore.getState().clear();
  useUIStore.getState().setTab("terminal");
  useApprovalStore.setState({ queue: [] });
  usePaletteStore.getState().close();
  useToastStore.setState({ toasts: [] });
}

function scenario(opts: Opts = {}): StoryObj {
  const { project = true, liveTerminal = project } = opts;
  return {
    loaders: [
      async () => {
        beginScenario(opts.fixtures, opts.theme);
        resetStores();
        if (opts.workbenchTab) {
          localStorage.setItem(`agent-console:workbench-tab:${ROOT}`, opts.workbenchTab);
        }
        if (project) await useSessionStore.getState().openProject(ROOT);
        return {};
      },
    ],
    play: async () => {
      // Let App's project effect hydrate sessions before touching them.
      await sleep(150);
      if (liveTerminal) {
        const st = useTerminalsStore.getState();
        const first = st.sessions[0];
        if (first) await st.resume(first.id);
      }
      if (opts.centerTab) useUIStore.getState().setTab(opts.centerTab);
      await opts.after?.();
      await settle();
    },
  };
}

const meta: Meta = {
  title: "App",
  component: App,
  parameters: { layout: "fullscreen" },
};
export default meta;

// ---- Shell -----------------------------------------------------------------

export const NoProject = scenario({ project: false });
export const Terminal = scenario({});
export const TerminalNoSessions = scenario({
  fixtures: { sessions_list: [] },
  liveTerminal: false,
});
export const Changes = scenario({ centerTab: "changes" });
export const Preview = scenario({ centerTab: "preview" });
export const LightTerminal = scenario({ theme: "light" });
export const LightChanges = scenario({ theme: "light", centerTab: "changes" });
export const OnboardingBanner = scenario({
  after: () => useOnboardingStore.setState({ bannerDismissed: false }),
});

// ---- Workbench: one story per tab ----------------------------------------

const wb: Record<string, StoryObj> = {};
for (const tab of WORKBENCH_TABS) {
  wb[tab] = scenario({ workbenchTab: tab });
}
export const WorkbenchTurns = wb.turns;
export const WorkbenchProof = wb.proof;
export const WorkbenchSkills = wb.skills;
export const WorkbenchAdvisor = wb.advisor;
export const WorkbenchLearning = wb.learning;
export const WorkbenchContext = wb.context;
export const WorkbenchRoundtable = wb.roundtable;
export const WorkbenchJobs = wb.jobs;
export const WorkbenchSchedule = wb.schedule;
export const WorkbenchPermissions = wb.permissions;
export const WorkbenchVault = wb.vault;
export const WorkbenchPlugins = wb.plugins;
export const WorkbenchMcp = wb.mcp;
export const WorkbenchJira = wb.jira;
export const WorkbenchTeams = wb.teams;
export const WorkbenchAgenda = wb.agenda;
export const WorkbenchNotes = wb.notes;
export const WorkbenchTransfer = wb.transfer;
export const WorkbenchFeedback = wb.feedback;

// ---- Overlays --------------------------------------------------------------

export const ApprovalModal = scenario({
  fixtures: { approvals_pending: [pendingApproval] },
  after: () => useApprovalStore.getState()._enqueue(pendingApproval),
});
export const CommandPalette = scenario({
  after: () => usePaletteStore.getState().openPalette(),
});
export const Shortcuts = scenario({
  after: () => {
    window.dispatchEvent(new CustomEvent("ac:open-shortcuts"));
  },
});
export const GettingStarted = scenario({
  after: () => {
    window.dispatchEvent(new CustomEvent("ac:open-getting-started"));
  },
});
export const WelcomeWizard = scenario({
  after: () => {
    window.dispatchEvent(new CustomEvent("ac:open-wizard"));
  },
});
export const Modules = scenario({
  after: () => {
    window.dispatchEvent(new CustomEvent("ac:open-modules"));
  },
});
export const ConfirmDialog = scenario({
  after: () => {
    void useConfirmStore.getState().ask({
      title: "Discard this worktree?",
      message:
        "ac/retry-path has 2 commits that are not on main.\nThe branch is kept; only the checkout is removed.",
      confirmLabel: "Discard",
      cancelLabel: "Keep",
    });
  },
});
export const Toasts = scenario({
  after: () => {
    const show = useToastStore.getState().show;
    show("Project path copied", "success");
    show("Cleaned 1 orphaned worktree (branches kept)", "info");
    show("Couldn't open https://example.com in your browser", "error");
  },
});
