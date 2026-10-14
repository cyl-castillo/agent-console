import { ipc } from "../ipc/tauri";
import { isValidModel, useModelStore } from "../stores/modelStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";
import type { AgentKind } from "../agents/profiles";
import type { LinkedFolder } from "../types/domain";

export interface WorktreePick {
  name: string;
  base: string;
}

/// Start a session the way the chooser describes it: in the project, in an
/// isolated worktree (created here), or in a linked folder. Remembers the
/// agent/model as the project's defaults. Returns false when the worktree
/// could not be created (the caller keeps its chooser open to retry).
export async function createSession(
  agent: AgentKind,
  model?: string,
  wt?: WorktreePick,
  folder?: LinkedFolder,
): Promise<boolean> {
  const project = useSessionStore.getState().project;
  if (!project) return false;
  const { add, persist } = useTerminalsStore.getState();
  const m = isValidModel(model) ? model : undefined;
  if (folder) {
    add(folder.path, undefined, m, agent);
  } else if (wt) {
    try {
      const created = await ipc.worktreeCreate(wt.name, wt.base);
      add(created.info.path, wt.name, m, agent, created.info, created.setupCommand ?? undefined);
      const copies = created.copied.length ? ` · copied ${created.copied.join(", ")}` : "";
      useToastStore.getState().show(`Worktree ready on ${created.info.branch}${copies}`, "success");
    } catch (e) {
      useToastStore.getState().show(`Couldn't create worktree: ${e}`, "error");
      return false;
    }
  } else {
    add(project.root, undefined, m, agent);
  }
  const models = useModelStore.getState();
  models.setDefaultFor(project.root, agent, m);
  models.setDefaultAgentFor(project.root, agent);
  useUIStore.getState().setTab("terminal");
  persist();
  return true;
}
