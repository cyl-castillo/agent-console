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

/// Every app-level modal, banner and toast, mounted once at the root.
/// Approvals and the palette only make sense with a project open.
export function OverlayHost({ withProject }: { withProject: boolean }) {
  const open = useOverlaysStore((s) => s.open);
  const hide = useOverlaysStore((s) => s.hide);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);

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
      {withProject && <ApprovalModal />}
      {withProject && <CommandPalette />}
      <ToastHost />
      <ConfirmDialog />
    </>
  );
}
