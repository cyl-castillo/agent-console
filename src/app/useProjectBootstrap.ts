import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ipc } from "../ipc/tauri";
import type { Project, WorkspaceContext } from "../types/domain";
import { useSessionStore } from "../stores/sessionStore";
import { useChangesStore } from "../stores/changesStore";
import { usePreviewStore } from "../stores/previewStore";
import { useSkillsStore } from "../stores/skillsStore";
import { useProofStore } from "../stores/proofStore";
import { useSchedulerStore } from "../stores/schedulerStore";
import { useInjectStore } from "../stores/injectStore";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { usePaletteStore } from "../stores/paletteStore";
import { useToastStore } from "../stores/toastStore";
import { useWorkbenchStore } from "../stores/workbenchStore";
import { maybeWorklogNudge } from "../lib/worklogNudge";
import { sessionCheckout } from "../lib/sessionCheckout";

export interface ProjectBootstrap {
  workspace: WorkspaceContext | null;
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  /// The active session's shell exited mid-run: no mounted terminal is
  /// visible, so the pane shows a "session ended" state with a resume button.
  activeStopped: boolean;
}

/// Everything that follows the open project: hydrate sessions, keep the
/// backend's active checkout in sync with the active session, refresh git /
/// skills / workspace, persist sessions on close and on a timer.
export function useProjectBootstrap(project: Project | null): ProjectBootstrap {
  const setTreeRoot = useSessionStore((s) => s.setTreeRoot);
  const checkoutEpoch = useSessionStore((s) => s.checkoutEpoch);
  const followProof = useProofStore((s) => s.followCheckout);
  const refreshChanges = useChangesStore((s) => s.refresh);
  const clearChanges = useChangesStore((s) => s.clear);
  const clearPreview = usePreviewStore((s) => s.clear);
  const refreshSkills = useSkillsStore((s) => s.refresh);
  const terminalSessions = useTerminalsStore((s) => s.sessions);
  const activeTerminalId = useTerminalsStore((s) => s.activeId);
  const hydrateTerminals = useTerminalsStore((s) => s.hydrate);
  const clearTerminals = useTerminalsStore((s) => s.clear);
  const persistTerminals = useTerminalsStore((s) => s.persist);
  const addTerminal = useTerminalsStore((s) => s.add);
  const resetPaletteForProject = usePaletteStore((s) => s.resetForProject);
  const restoreWorkbench = useWorkbenchStore((s) => s.restoreFor);
  const showToast = useToastStore((s) => s.show);
  const [workspace, setWorkspace] = useState<WorkspaceContext | null>(null);

  // Morning worklog nudge: once per day, if yesterday's witnessed ticket time
  // was never logged, say so (Tasks → Witnessed time). Courtesy only.
  useEffect(() => {
    if (project?.root) void maybeWorklogNudge(project.root);
  }, [project?.root]);

  // Memory-injection state for this project (toggle + recent injections).
  useEffect(() => {
    if (project?.root) void useInjectStore.getState().load(project.root);
  }, [project?.root]);

  // Keep the backend's notion of "the checkout being worked on" in sync with
  // the active session: git/snapshot commands, the change watcher, the file
  // tree and the Proof/Turns ledger follow the session's isolated worktree or
  // linked folder (or the project root). Refresh Changes after the switch so
  // the view never shows the previous checkout's status.
  const activeSession = terminalSessions.find((s) => s.id === activeTerminalId);
  const activeCheckout = project ? sessionCheckout(activeSession, project.root) : null;
  useEffect(() => {
    if (!project) return;
    ipc
      .setActiveRepo(activeCheckout)
      .then(() => {
        refreshChanges();
        void setTreeRoot(activeCheckout);
        void followProof(activeCheckout ?? project.root);
      })
      .catch(() => {
        // Stale session: its worktree was removed or its folder is no longer
        // linked. A rejected path leaves the previous override in place, so
        // reset it explicitly — never keep pointing at the last session's folder.
        void ipc.setActiveRepo(null).then(
          () => refreshChanges(),
          () => {},
        );
        void setTreeRoot(null);
        void followProof(project.root);
      });
    // checkoutEpoch: re-check when a folder is unlinked under the active session.
  }, [project, activeCheckout, checkoutEpoch, refreshChanges, setTreeRoot, followProof]);

  // Reload git/skills/workspace and hydrate sessions when project changes.
  useEffect(() => {
    if (!project) {
      clearChanges();
      clearPreview();
      clearTerminals();
      setWorkspace(null);
      resetPaletteForProject(null);
      restoreWorkbench(null);
      return;
    }
    refreshChanges();
    refreshSkills();
    void useSchedulerStore.getState().refresh();
    resetPaletteForProject(project.root);
    // Palette file index is built lazily on first palette open (see
    // paletteStore.openPalette) — not eagerly here, to keep the full-tree
    // walk off the project-open path.
    restoreWorkbench(project.root);
    ipc
      .workspaceContext()
      .then(setWorkspace)
      .catch(() => setWorkspace(null));
    (async () => {
      await hydrateTerminals(project.root);
      // Auto-spawn one live session only when the load SUCCEEDED and restored
      // nothing. If hydrate failed (ready=false) we must not spawn — doing so
      // would let the next persist overwrite history we simply couldn't read.
      const st = useTerminalsStore.getState();
      if (st.ready && st.sessions.length === 0) {
        addTerminal(project.root);
      }
      // Clean up managed worktree checkouts no session references any more
      // (branches are kept). Only when hydrate succeeded — a failed load must
      // not be allowed to orphan every live worktree.
      if (st.ready) {
        const keep = st.sessions
          .map((s) => s.worktree?.path)
          .filter((p): p is string => Boolean(p));
        ipc
          .worktreePruneOrphans(keep)
          .then((removed) => {
            if (removed.length > 0) {
              showToast(
                `Cleaned ${removed.length} orphaned worktree${removed.length === 1 ? "" : "s"} (branches kept)`,
                "info",
              );
            }
          })
          .catch(() => {
            /* best-effort */
          });
      }
    })();
  }, [
    project,
    refreshChanges,
    refreshSkills,
    clearChanges,
    clearPreview,
    hydrateTerminals,
    clearTerminals,
    addTerminal,
    resetPaletteForProject,
    restoreWorkbench,
    showToast,
  ]);

  // Persist sessions when the app closes. beforeunload alone is NOT enough:
  // on Windows/WebView2 the window can be destroyed without dispatching it
  // (issue #72 — sessions lost on close/update). Tauri's close-requested hook
  // is the reliable path: hold the close, await the persist, then destroy.
  useEffect(() => {
    const onUnload = () => {
      persistTerminals();
    };
    window.addEventListener("beforeunload", onUnload);

    let unlisten: (() => void) | null = null;
    let disposed = false;
    let closing = false;
    void getCurrentWindow()
      .onCloseRequested(async (event) => {
        // Second pass (our close() fallback re-entering): let the default
        // close proceed — never block the user's X twice.
        if (closing) return;
        closing = true;
        event.preventDefault();
        try {
          // Bounded: a wedged IPC must not hold the window hostage. The
          // periodic 10s persist is the backstop for whatever this misses.
          await Promise.race([
            persistTerminals(),
            new Promise((resolve) => setTimeout(resolve, 2000)),
          ]);
        } catch {
          /* closing anyway */
        }
        // destroy() skips close-requested (needs core:window:allow-destroy);
        // if it's unavailable for any reason, close() re-enters this handler
        // with closing=true and falls through to the default close.
        getCurrentWindow()
          .destroy()
          .catch(() => void getCurrentWindow().close());
      })
      .then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    return () => {
      window.removeEventListener("beforeunload", onUnload);
      disposed = true;
      unlisten?.();
    };
  }, [persistTerminals]);

  // Periodic persist so a hard kill doesn't lose more than ~10s of scrollback.
  useEffect(() => {
    if (!project) return;
    const t = setInterval(() => {
      persistTerminals();
    }, 10_000);
    return () => clearInterval(t);
  }, [project, persistTerminals]);

  const liveTerminals = terminalSessions.filter((s) => s.status === "live");
  const activeTerminal = terminalSessions.find((s) => s.id === activeTerminalId);
  const activeStopped = activeTerminal?.status === "stopped" && !activeTerminal.archived;

  return { workspace, liveTerminals, activeTerminal, activeStopped };
}
