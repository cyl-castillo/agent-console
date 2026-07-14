import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { StickyNote } from "../types/domain";

export const NOTE_COLORS = ["yellow", "pink", "blue", "green", "purple"] as const;

function genId(): string {
  return `note-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/// Debounce handle for text-edit persists (module-level: one board at a time).
let persistTimer: ReturnType<typeof setTimeout> | null = null;

interface NotesState {
  projectRoot: string | null;
  notes: StickyNote[];
  loading: boolean;
  error: string | null;

  load: (projectRoot: string) => Promise<void>;
  clear: () => void;
  add: () => void;
  updateText: (id: string, text: string) => void;
  setColor: (id: string, color: string) => void;
  remove: (id: string) => void;
  /// Persist the current list. Called by every mutation (autosave on the
  /// mutation that produces data — a crash before close still persists).
  persist: () => Promise<void>;
}

export const useNotesStore = create<NotesState>((set, get) => ({
  projectRoot: null,
  notes: [],
  loading: false,
  error: null,

  load: async (projectRoot) => {
    set({ loading: true, error: null, projectRoot });
    try {
      const notes = await ipc.notesList(projectRoot);
      set({ notes, loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  clear: () => set({ projectRoot: null, notes: [], error: null }),

  add: () => {
    const now = Date.now();
    const note: StickyNote = {
      id: genId(),
      text: "",
      // Rotate through the palette so adjacent new notes differ at a glance.
      color: NOTE_COLORS[get().notes.length % NOTE_COLORS.length],
      createdAtMs: now,
      updatedAtMs: now,
    };
    // Newest first — a scratchpad's most recent thought belongs on top.
    set((s) => ({ notes: [note, ...s.notes] }));
    void get().persist();
  },

  updateText: (id, text) => {
    set((s) => ({
      notes: s.notes.map((n) => (n.id === id ? { ...n, text, updatedAtMs: Date.now() } : n)),
    }));
    // Typing fires this per keystroke — debounce the disk write. Structural
    // mutations (add/remove/color) persist immediately.
    if (persistTimer !== null) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => {
      persistTimer = null;
      void get().persist();
    }, 600);
  },

  setColor: (id, color) => {
    set((s) => ({
      notes: s.notes.map((n) => (n.id === id ? { ...n, color, updatedAtMs: Date.now() } : n)),
    }));
    void get().persist();
  },

  remove: (id) => {
    set((s) => ({ notes: s.notes.filter((n) => n.id !== id) }));
    void get().persist();
  },

  persist: async () => {
    const { projectRoot, notes } = get();
    if (!projectRoot) return;
    try {
      await ipc.notesSave(projectRoot, notes);
    } catch (e) {
      set({ error: String(e) });
    }
  },
}));
