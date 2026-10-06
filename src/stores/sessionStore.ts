import { create } from "zustand";
import type { FileNode, Project } from "../types/domain";
import { ipc } from "../ipc/tauri";

interface SessionState {
  project: Project | null;
  tree: FileNode | null;
  /// Folder the file tree shows: the active session's checkout (worktree or
  /// linked folder). Null = the project root.
  treeRoot: string | null;
  loading: boolean;
  error: string | null;

  openProject: (path: string) => Promise<void>;
  closeProject: () => void;
  refreshTree: () => Promise<void>;
  /// Point the file tree at `root` (null = back to the project root).
  setTreeRoot: (root: string | null) => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  project: null,
  tree: null,
  treeRoot: null,
  loading: false,
  error: null,

  openProject: async (path) => {
    set({ loading: true, error: null });
    try {
      const project = await ipc.openProject(path);
      const tree = await ipc.readTree(project.root, 3);
      set({ project, tree, treeRoot: null, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  closeProject: () => set({ project: null, tree: null, treeRoot: null, error: null }),

  refreshTree: async () => {
    const { project, treeRoot } = get();
    if (!project) return;
    const root = treeRoot ?? project.root;
    try {
      const tree = await ipc.readTree(root, 3);
      // A session switch while this was in flight wins.
      if ((get().treeRoot ?? get().project?.root) === root) set({ tree });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  setTreeRoot: async (root) => {
    const { project, treeRoot } = get();
    if (!project) return;
    const next = root && root !== project.root ? root : null;
    if (next === treeRoot) return;
    set({ treeRoot: next });
    await get().refreshTree();
  },
}));
