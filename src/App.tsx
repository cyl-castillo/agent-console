import { ProjectPicker } from "./components/ProjectPicker";
import { useSessionStore } from "./stores/sessionStore";
import { useUIStore } from "./stores/uiStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useVoicePtt } from "./hooks/useVoicePtt";
import { OverlayHost } from "./app/OverlayHost";
import { ShellV2 } from "./app/v2/ShellV2";
import { useAppLifecycle, useWindowTitle } from "./app/useAppLifecycle";
import { useOnboardingAutoOpen } from "./app/useOnboardingAutoOpen";
import { useProjectBootstrap } from "./app/useProjectBootstrap";

export default function App() {
  const project = useSessionStore((s) => s.project);
  const setTab = useUIStore((s) => s.setTab);

  useAppLifecycle();
  useWindowTitle();
  useOnboardingAutoOpen();
  const boot = useProjectBootstrap(project);
  useKeyboardShortcuts({ setTab });
  useVoicePtt();

  if (!project) {
    return (
      <>
        <ProjectPicker />
        <OverlayHost withProject={false} />
      </>
    );
  }

  return (
    <>
      <ShellV2 project={project} {...boot} />
      <OverlayHost withProject />
    </>
  );
}
