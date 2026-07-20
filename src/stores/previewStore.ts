import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import { useChangesStore } from "./changesStore";
import { useSessionStore } from "./sessionStore";

/// Preview tab state. A file is either rendered as a unified diff (when it
/// has uncommitted changes) or as read-only text.
interface PreviewState {
  /// Absolute path of the file currently shown.
  selectedAbs: string | null;
  /// Path as displayed in the header (relative to project root when possible).
  displayPath: string | null;
  mode: "empty" | "content" | "diff" | "binary";
  content: string;
  sizeBytes: number;
  truncated: boolean;
  loading: boolean;
  error: string | null;

  open: (absPath: string) => Promise<void>;
  clear: () => void;
}

export const usePreviewStore = create<PreviewState>((set) => ({
  selectedAbs: null,
  displayPath: null,
  mode: "empty",
  content: "",
  sizeBytes: 0,
  truncated: false,
  loading: false,
  error: null,

  open: async (absPath) => {
    const project = useSessionStore.getState().project;
    const root = project?.root ?? "";
    const relative = absPath.startsWith(root + "/")
      ? absPath.slice(root.length + 1)
      : absPath === root
        ? ""
        : absPath;

    set({
      selectedAbs: absPath,
      displayPath: relative || absPath,
      loading: true,
      error: null,
    });

    // If the file is in the git changes list, show its diff.
    const changes = useChangesStore.getState().status?.changes ?? [];
    const change = changes.find((c) => c.path === relative);
    if (change) {
      try {
        const diff = await ipc.gitDiffFile(relative);
        set({ mode: "diff", content: diff, loading: false });
      } catch (e) {
        set({ error: String(e), loading: false });
      }
      return;
    }

    // Otherwise read the file content.
    try {
      const fc = await ipc.readFileText(absPath);
      set({
        mode: fc.isBinary ? "binary" : "content",
        content: fc.content,
        sizeBytes: fc.sizeBytes,
        truncated: fc.truncated,
        loading: false,
      });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  clear: () =>
    set({
      selectedAbs: null,
      displayPath: null,
      mode: "empty",
      content: "",
      sizeBytes: 0,
      truncated: false,
      error: null,
    }),
}));
