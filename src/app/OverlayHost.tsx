import { AboutModal } from "../components/AboutModal";
import { ApprovalModal } from "../components/ApprovalModal";
import { CommandPalette } from "../components/CommandPalette";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { GettingStartedModal } from "../components/GettingStartedModal";
import { ModulesModal } from "../components/ModulesModal";
import { ShortcutsModal } from "../components/ShortcutsModal";
import { UpdateBanner } from "../components/UpdateBanner";
import { WelcomeWizard } from "../components/WelcomeWizard";
import { useOverlaysStore } from "../stores/overlaysStore";
import { useWorkbenchStore } from "../stores/workbenchStore";
import { ToastHost } from "../ui/Toast";
import { useShellStore } from "../stores/shellStore";
import { useApprovalStore } from "../stores/approvalStore";
import { useTerminalsStore } from "../stores/terminalsStore";

/// Every app-level modal, banner and toast, mounted once at the root.
/// Approvals and the palette only make sense with a project open.
export function OverlayHost({ withProject }: { withProject: boolean }) {
  const open = useOverlaysStore((s) => s.open);
  const hide = useOverlaysStore((s) => s.hide);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  const destination = useShellStore((s) => s.destination);
  const queue = useApprovalStore((s) => s.queue);
  const activeId = useTerminalsStore((s) => s.activeId);
  // The Inbox IS the decision surface, and in Sessions the active session's
  // approvals sit inline in its conversation (F3): the modal only pops when
  // a decision would otherwise be out of reach.
  const inlineCovers =
    destination === "sessions" && queue.length > 0 && queue.every((r) => r.termId === activeId);
  const modalApprovals = destination !== "inbox" && !inlineCovers;

  return (
    <>
      {open.about && <AboutModal onClose={() => hide("about")} />}
      {open.gettingStarted && (
        <GettingStartedModal onClose={() => hide("gettingStarted")} onJumpToTab={setWorkbenchTab} />
      )}
      {open.shortcuts && <ShortcutsModal onClose={() => hide("shortcuts")} />}
      {withProject && open.modules && <ModulesModal onClose={() => hide("modules")} />}
      {open.wizard && <WelcomeWizard onClose={() => hide("wizard")} />}
      <UpdateBanner />
      {withProject && modalApprovals && <ApprovalModal />}
      {withProject && <CommandPalette />}
      <ToastHost />
      <ConfirmDialog />
    </>
  );
}
