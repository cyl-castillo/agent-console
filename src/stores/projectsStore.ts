import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { RecentProject } from "../types/domain";

interface ProjectsState {
  recent: RecentProject[];
  load: () => Promise<void>;
  forget: (path: string) => Promise<void>;
}

export const useProjectsStore = create<ProjectsState>((set, get) => ({
  recent: [],
  load: async () => {
    try {
      const recent = await ipc.projectsRecent();
      set({ recent });
    } catch {
      /* ignore */
    }
  },
  forget: async (path) => {
    try {
      await ipc.projectsForget(path);
      await get().load();
    } catch {
      /* ignore */
    }
  },
}));
