import { create } from "zustand";
import type { FileNode, Project } from "../types/domain";
import { ipc } from "../ipc/tauri";

interface SessionState {
  project: Project | null;
  tree: FileNode | null;
  loading: boolean;
  error: string | null;

  openProject: (path: string) => Promise<void>;
  closeProject: () => void;
  refreshTree: () => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  project: null,
  tree: null,
  loading: false,
  error: null,

  openProject: async (path) => {
    set({ loading: true, error: null });
    try {
      const project = await ipc.openProject(path);
      const tree = await ipc.readTree(project.root, 3);
      set({ project, tree, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  closeProject: () => set({ project: null, tree: null, error: null }),

  refreshTree: async () => {
    const { project } = get();
    if (!project) return;
    try {
      const tree = await ipc.readTree(project.root, 3);
      set({ tree });
    } catch (err) {
      set({ error: String(err) });
    }
  },
}));
