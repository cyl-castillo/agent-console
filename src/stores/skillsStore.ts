import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { HooksStatus, HookUserPromptEvent, Skill, Snapshot } from "../types/domain";
import { useChangesStore } from "./changesStore";

/// What the user (or agent in the terminal) has been doing — captured from
/// the UserPromptSubmit hook stream.
export interface PromptEvent {
  id: string;
  ts: number;
  prompt: string;
  skill?: string;
  snapshotCommitSha?: string;
}

const MAX_RECENT = 30;

interface SkillsState {
  installed: Skill[];
  recent: PromptEvent[];
  hooks: HooksStatus | null;
  selected: Skill | null;
  selectedMarkdown: string;

  refresh: () => Promise<void>;
  install: () => Promise<void>;
  uninstall: () => Promise<void>;
  open: (skill: Skill | null) => Promise<void>;
  restoreSnapshot: (commitSha: string) => Promise<void>;

  _onPrompt: (e: HookUserPromptEvent) => void;
  _onSnapshot: (snap: Snapshot) => void;
}

export const useSkillsStore = create<SkillsState>((set, get) => ({
  installed: [],
  recent: [],
  hooks: null,
  selected: null,
  selectedMarkdown: "",

  refresh: async () => {
    try {
      const [skills, status] = await Promise.all([
        ipc.skillList(),
        ipc.hooksStatus(),
      ]);
      set({ installed: skills, hooks: status });
    } catch { /* ignore */ }
  },

  install: async () => {
    try {
      const status = await ipc.hooksInstall();
      set({ hooks: status });
    } catch { /* ignore */ }
  },

  uninstall: async () => {
    try {
      const status = await ipc.hooksUninstall();
      set({ hooks: status });
    } catch { /* ignore */ }
  },

  open: async (skill) => {
    set({ selected: skill, selectedMarkdown: "" });
    if (!skill) return;
    try {
      const md = await ipc.skillRead(skill.path);
      if (get().selected?.path === skill.path) set({ selectedMarkdown: md });
    } catch { /* ignore */ }
  },

  restoreSnapshot: async (commitSha) => {
    try {
      await ipc.snapshotRestore(commitSha);
      await useChangesStore.getState().refresh();
    } catch { /* ignore */ }
  },

  _onPrompt: (e) => {
    const entry: PromptEvent = {
      id: crypto.randomUUID(),
      ts: e.ts,
      prompt: e.prompt,
      skill: e.skill,
    };
    set((s) => ({ recent: [entry, ...s.recent].slice(0, MAX_RECENT) }));
  },

  _onSnapshot: (snap) => {
    // Attach the snapshot sha to the most-recent prompt (if any).
    set((s) => {
      const [first, ...rest] = s.recent;
      if (!first || first.snapshotCommitSha) return s;
      return { recent: [{ ...first, snapshotCommitSha: snap.commitSha }, ...rest] };
    });
  },
}));

export async function attachSkillsListeners(): Promise<UnlistenFn> {
  const s = useSkillsStore.getState();
  const offs: UnlistenFn[] = [];
  offs.push(await listen<HookUserPromptEvent>("hook://user_prompt", (e) => s._onPrompt(e.payload)));
  offs.push(await listen<Snapshot>("snapshot://created", (e) => s._onSnapshot(e.payload)));
  return () => { for (const off of offs) off(); };
}
