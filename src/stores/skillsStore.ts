import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { HooksStatus, HookUserPromptEvent, Skill, Snapshot } from "../types/domain";
import { useChangesStore } from "./changesStore";
import { useOnboardingStore } from "./onboardingStore";
import { useTerminalsStore } from "./terminalsStore";

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

/// Turn a user prompt into a short, file-name-ish label for the session row.
/// Local & instant — first ~5 meaningful words, trimmed, capitalised.
function deriveSessionLabel(prompt: string): string {
  if (!prompt) return "";
  let s = prompt
    .replace(/```[\s\S]*?```/g, " ")  // drop fenced code blocks
    .replace(/`[^`]*`/g, " ")          // drop inline code
    .replace(/https?:\/\/\S+/g, " ")   // drop urls
    .replace(/[#*_>`~|]/g, " ")        // drop markdown punctuation
    .replace(/[\r\n]+/g, " ")
    .trim();
  // Cut at the first sentence boundary if present.
  const m = s.match(/^[^.!?\n]{3,}/);
  if (m) s = m[0];
  const words = s.split(/\s+/).filter(Boolean).slice(0, 5);
  let label = words.join(" ").slice(0, 40).trim();
  if (!label) return "";
  // Capitalise first letter, leave the rest as the user wrote it.
  label = label.charAt(0).toUpperCase() + label.slice(1);
  return label;
}

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
    useOnboardingStore.getState().markPromptedClaude();

    // Associate the Claude session id with whatever terminal is active when the
    // prompt fires. This is best-effort: there's no field in the hook payload
    // pointing back to a specific PTY, so we assume the active terminal is the
    // one running claude.
    if (e.sessionId) {
      const { activeId } = useTerminalsStore.getState();
      if (activeId) {
        useTerminalsStore.getState().setClaudeSessionId(activeId, e.sessionId);
        // Offer a meaningful rename derived from the first prompt — only if
        // this is still the first prompt (the session had no claudeSessionId
        // before we just set it).
        const target = useTerminalsStore.getState().sessions.find((s) => s.id === activeId);
        const hadNoPriorClaude = !!target && target.claudeSessionId === e.sessionId
          && !target.suggestedName; // not yet suggested
        if (hadNoPriorClaude) {
          const label = deriveSessionLabel(e.prompt);
          if (label && label !== target.name) {
            useTerminalsStore.getState().suggestName(activeId, label);
          }
        }
      }
    }
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
