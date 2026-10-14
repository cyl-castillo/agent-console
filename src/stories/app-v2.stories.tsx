// The v2 shell (flag ui.v2): the same fixtures as App/*, rendered through the
// rail → list → centre → inspector layout. Compared against their own
// baseline; the v1 stories stay untouched.
import type { Meta, StoryObj } from "@storybook/react-vite";
import App from "../App";
import { approvalEdit, approvalHigh, pendingApproval, ROOT } from "../dev/fixtures/defaults";
import { useBudgetStore } from "../stores/budgetStore";
import { useLiveStatusStore } from "../stores/liveStatusStore";
import { useDeclinesStore } from "../stores/declinesStore";
import type { FixtureMap } from "../dev/fixtures/registry";
import { useApprovalStore } from "../stores/approvalStore";
import { useFlagsStore } from "../stores/flagsStore";
import { usePaletteStore } from "../stores/paletteStore";
import { useSessionStore } from "../stores/sessionStore";
import { useShellStore } from "../stores/shellStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";
import { useWorkbenchStore } from "../stores/workbenchStore";
import type { WorkbenchTab } from "../lib/workbenchTabs";
import type { Destination } from "../app/v2/destinations";
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
        useFlagsStore.getState().set("ui.v2", true);
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
  fixtures: { approvals_pending: [pendingApproval, approvalHigh, approvalEdit] },
  after: () => {
    useApprovalStore.setState({ queue: [pendingApproval, approvalHigh, approvalEdit] });
    const b = useBudgetStore.getState();
    b.setCap("sess-auth", 5);
    b.markBreached("sess-auth", 5.12);
    useLiveStatusStore
      .getState()
      .note({ ts: Date.now(), termId: "sess-auth", costUsd: 5.12, usedPct: 31 });
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
export const Sessions = scenario({ destination: "sessions" });
export const SessionsDrawerOpen = scenario({
  destination: "sessions",
  after: () => {
    if (!useShellStore.getState().drawerOpen) useShellStore.getState().toggleDrawer();
  },
});
export const SessionsNoTurns = scenario({
  destination: "sessions",
  fixtures: { testigo_list: [] },
});
export const SessionsChanges = scenario({ destination: "sessions", centerTab: "changes" });
export const WorkJobs = scenario({ destination: "work", tab: "jobs" });
export const WorkRoom = scenario({ destination: "work", tab: "roundtable" });
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
