import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { attachSkillsListeners } from "../stores/skillsStore";
import { attachApprovalListener, useApprovalStore } from "../stores/approvalStore";
import { attachGitWatcherListener } from "../stores/changesStore";
import { attachVoiceListeners, attachVoiceApprovalWatcher } from "../stores/voiceStore";
import { attachSchedulerListeners } from "../stores/schedulerStore";
import { attachInjectListener } from "../stores/injectStore";
import { attachHooksHealthListeners } from "../stores/hooksHealthStore";
import { attachLiveStatusListener } from "../stores/liveStatusStore";
import { attachProofListeners } from "../stores/proofStore";
import { attachWorkbenchListeners } from "../stores/workbenchStore";
import { attachOverlayListeners } from "../stores/overlaysStore";
import { attachConsoleMcpHost } from "../lib/consoleMcpHost";
import { attachDeclineListener } from "../stores/declinesStore";
import { attachHandoffWatcher } from "../stores/handoffStore";
import { useToastStore } from "../stores/toastStore";
import { useUpdaterStore } from "../stores/updaterStore";
import { useFeedbackStore } from "../stores/feedbackStore";
import { useUIStore } from "../stores/uiStore";
import { adoptLiveResumeHandles, RESUME_HANDLE_POLL_MS } from "../lib/resumeHandles";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { copyProjectPath, newSession } from "./actions";

/// Everything the app wires up exactly once per mount, independent of which
/// project is open: backend event listeners, palette window events, the
/// periodic update check, the resume-handle poll.
export function useAppLifecycle(): void {
  const checkForUpdates = useUpdaterStore((s) => s.check);
  const initFeedback = useFeedbackStore((s) => s.init);

  // Backend event listeners. If cleanup runs before a listen() promise
  // resolves (StrictMode double mount, fast unmount), unlisten immediately
  // instead of leaking it.
  useEffect(() => {
    let disposed = false;
    const offs: (() => void)[] = [];
    const adopt = (p: Promise<() => void>) => {
      p.then((u) => {
        if (disposed) u();
        else offs.push(u);
      });
    };
    adopt(attachSkillsListeners());
    adopt(attachApprovalListener());
    adopt(attachGitWatcherListener());
    // Corrupt session history was quarantined so saving could resume — the
    // one situation that must never pass silently again (#72).
    adopt(
      listen<{ path: string }>("sessions://quarantined", (e) => {
        useToastStore
          .getState()
          .show(
            `Session history file was corrupt; it was set aside at ${e.payload.path} and saving has resumed.`,
            "error",
          );
      }),
    );
    adopt(attachVoiceListeners());
    adopt(attachSchedulerListeners());
    adopt(attachInjectListener());
    adopt(attachHooksHealthListeners());
    adopt(attachLiveStatusListener());
    adopt(attachProofListeners());
    // The console as an MCP server: answer the Rust side's roundtrips.
    adopt(attachConsoleMcpHost());
    adopt(attachDeclineListener());
    offs.push(attachHandoffWatcher());
    offs.push(attachVoiceApprovalWatcher());
    offs.push(attachWorkbenchListeners());
    offs.push(attachOverlayListeners());
    return () => {
      disposed = true;
      offs.forEach((u) => u());
    };
  }, []);

  // Palette-triggered navigation events.
  useEffect(() => {
    const ui = () => useUIStore.getState();
    const onOpenTab = (e: Event) => {
      const d = (e as CustomEvent).detail;
      if (d === "terminal" || d === "changes" || d === "preview") ui().setTab(d);
    };
    const onToggleSidebar = () => ui().toggleLeft();
    const onToggleRight = () => ui().toggleRight();
    const onNewSession = () => newSession();
    const onCopyProjectPath = () => copyProjectPath();
    const onToggleComposer = () => ui().toggleComposer();
    window.addEventListener("ac:toggle-composer", onToggleComposer);
    window.addEventListener("ac:open-tab", onOpenTab);
    window.addEventListener("ac:toggle-sidebar", onToggleSidebar);
    window.addEventListener("ac:toggle-right-panel", onToggleRight);
    window.addEventListener("ac:new-session", onNewSession);
    window.addEventListener("ac:copy-project-path", onCopyProjectPath);
    return () => {
      window.removeEventListener("ac:open-tab", onOpenTab);
      window.removeEventListener("ac:toggle-sidebar", onToggleSidebar);
      window.removeEventListener("ac:toggle-right-panel", onToggleRight);
      window.removeEventListener("ac:new-session", onNewSession);
      window.removeEventListener("ac:copy-project-path", onCopyProjectPath);
      window.removeEventListener("ac:toggle-composer", onToggleComposer);
    };
  }, []);

  useEffect(() => {
    checkForUpdates({ silentIfNone: true });
    // Re-check periodically: this app commonly stays open for days, so a
    // startup-only check means a long-running session never learns about a new
    // release. Silent unless something's actually available.
    const id = window.setInterval(
      () => checkForUpdates({ silentIfNone: true }),
      6 * 60 * 60 * 1000,
    );
    return () => window.clearInterval(id);
  }, [checkForUpdates]);

  useEffect(() => {
    void initFeedback();
  }, [initFeedback]);

  // Terminals whose agent never went through a hook still learn their resume
  // handle: `claude agents --json` says which session runs in which process,
  // and the backend matches those against each PTY's shell. Costs nothing once
  // every live terminal has an id (the pass stops before the IPC).
  useEffect(() => {
    const id = window.setInterval(() => {
      void adoptLiveResumeHandles();
    }, RESUME_HANDLE_POLL_MS);
    return () => window.clearInterval(id);
  }, []);
}

/// Mirror the "waiting on you" state into the window title, so the taskbar /
/// Alt-Tab entry says it even when the app isn't visible.
export function useWindowTitle(): void {
  const blockedCount = useApprovalStore((s) => s.queue.length);
  useEffect(() => {
    const title =
      blockedCount > 0 ? `● waiting (${blockedCount}) — Agent Console` : "Agent Console";
    getCurrentWindow()
      .setTitle(title)
      .catch(() => {
        /* cosmetic — never break on it */
      });
  }, [blockedCount]);
}
