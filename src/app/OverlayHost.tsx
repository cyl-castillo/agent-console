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
import { useFlagsStore } from "../stores/flagsStore";
import { useShellStore } from "../stores/shellStore";

/// Every app-level modal, banner and toast, mounted once at the root.
/// Approvals and the palette only make sense with a project open.
export function OverlayHost({ withProject }: { withProject: boolean }) {
  const open = useOverlaysStore((s) => s.open);
  const hide = useOverlaysStore((s) => s.hide);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  const v2 = useFlagsStore((s) => s.flags["ui.v2"]);
  const destination = useShellStore((s) => s.destination);
  // In the v2 shell the Inbox IS the decision surface: the modal only pops
  // when you are somewhere else (so a decision is never out of reach).
  const modalApprovals = !(v2 && destination === "inbox");

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
