// The v2 shell (flag ui.v2): the same fixtures as App/*, rendered through the
// rail → list → centre → inspector layout. Compared against their own
// baseline; the v1 stories stay untouched.
import type { Meta, StoryObj } from "@storybook/react-vite";
import App from "../App";
import { pendingApproval, ROOT } from "../dev/fixtures/defaults";
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
export const InboxEmpty = scenario({
  fixtures: {
    sessions_list: [],
    jobs_board: { project: ROOT, settings: { parallelJobs: 1 }, cards: [], pending: [], busy: 0 },
  },
  liveTerminal: false,
});
export const Sessions = scenario({ destination: "sessions" });
export const SessionsChanges = scenario({ destination: "sessions", centerTab: "changes" });
export const WorkJobs = scenario({ destination: "work", tab: "jobs" });
export const WorkRoom = scenario({ destination: "work", tab: "roundtable" });
export const ProofLedger = scenario({ destination: "proof", tab: "proof" });
export const SettingsPermissions = scenario({ destination: "settings", tab: "permissions" });
export const SettingsSkills = scenario({ destination: "settings", tab: "skills" });
export const LightInbox = scenario({ theme: "light" });
export const LightSessions = scenario({ theme: "light", destination: "sessions" });
