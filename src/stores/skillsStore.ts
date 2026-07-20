import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { HooksStatus, HookUserPromptEvent, Skill, Snapshot } from "../types/domain";
import { useChangesStore } from "./changesStore";
import { useLearningStore } from "./learningStore";
import { useOnboardingStore } from "./onboardingStore";
import { fireSchedulerEvent } from "./schedulerStore";
import { useToastStore } from "./toastStore";
import { useAgentStatusStore } from "./agentStatusStore";
import { useTerminalsStore } from "./terminalsStore";
import { notify, windowIsFocused } from "../lib/notify";

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

/// Tracks the project skill count across refreshes so we can fire the
/// "corpus_grew" scheduler event only when a skill is actually added (not on the
/// first load, and not on removals). -1 = no baseline yet.
let lastSkillCount = -1;

/// Turn a user prompt into a short, file-name-ish label for the session row.
/// Local & instant — first ~5 meaningful words, trimmed, capitalised.
function deriveSessionLabel(prompt: string): string {
  if (!prompt) return "";
  let s = prompt
    .replace(/```[\s\S]*?```/g, " ") // drop fenced code blocks
    .replace(/`[^`]*`/g, " ") // drop inline code
    .replace(/https?:\/\/\S+/g, " ") // drop urls
    .replace(/[#*_>`~|]/g, " ") // drop markdown punctuation
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
  /** Backup taken before the last restore, so "undo last restore" can re-apply it. */
  undoRestoreSha: string | null;

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
  undoRestoreSha: null,

  refresh: async () => {
    try {
      const [skills, status] = await Promise.all([ipc.skillList(), ipc.hooksStatus()]);
      set({ installed: skills, hooks: status });
      // Corpus changed → maybe the curator should tidy it (threshold auto-trigger).
      useLearningStore.getState().noteCorpusSize();
      // Notify scheduler jobs watching for new skills (only on real growth).
      const n = skills.filter((sk) => sk.source === "project" && sk.kind === "skill").length;
      if (lastSkillCount >= 0 && n > lastSkillCount) void fireSchedulerEvent("corpus_grew");
      lastSkillCount = n;
    } catch (e) {
      console.error("[skills] refresh failed:", e);
    }
  },

  install: async () => {
    try {
      const status = await ipc.hooksInstall();
      set({ hooks: status });
    } catch (e) {
      console.error("[skills] install failed:", e);
    }
  },

  uninstall: async () => {
    try {
      const status = await ipc.hooksUninstall();
      set({ hooks: status });
    } catch (e) {
      console.error("[skills] uninstall failed:", e);
    }
  },

  open: async (skill) => {
    set({ selected: skill, selectedMarkdown: "" });
    if (!skill) return;
    try {
      const md = await ipc.skillRead(skill.path);
      if (get().selected?.path === skill.path) set({ selectedMarkdown: md });
    } catch (e) {
      console.error("[skills] open failed:", e);
    }
  },

  restoreSnapshot: async (commitSha) => {
    try {
      // The backend captures a pre-restore backup and returns its sha, so the
      // destructive restore is itself undoable. Stash it for "undo last restore".
      const undoSha = await ipc.snapshotRestore(commitSha);
      set({ undoRestoreSha: undoSha ?? null });
      await useChangesStore.getState().refresh();
      useToastStore
        .getState()
        .show(undoSha ? "Restored. Undo via ⌘P → Undo last restore" : "Restored", "success");
    } catch (e) {
      useToastStore.getState().show(`Restore failed: ${e}`, "error");
    }
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
    // A submitted prompt means a turn just started — light the "working" pill.
    useAgentStatusStore.getState().markActive();
    // Feed the learning auto-trigger: enough new activity reflects on its own.
    useLearningStore.getState().noteActivity();
    // Notify scheduler jobs watching for prompts.
    void fireSchedulerEvent("prompt");

    // Associate the Claude session id with the terminal that emitted the prompt.
    // The hook tags each prompt with the PTY's terminal-session id (termId, from
    // AGENT_CONSOLE_TERM_ID), so we can bind deterministically — even when several
    // claude sessions run at once. We only fall back to the active terminal when
    // termId is missing (e.g. a claude launched before this build's hook change).
    if (e.sessionId) {
      const { activeId, sessions } = useTerminalsStore.getState();
      const targetId = e.termId && sessions.some((s) => s.id === e.termId) ? e.termId : activeId;
      if (targetId) {
        useTerminalsStore.getState().setClaudeSessionId(targetId, e.sessionId);
        // Offer a meaningful rename derived from the first prompt — only if
        // this is still the first prompt (the session had no claudeSessionId
        // before we just set it).
        const target = useTerminalsStore.getState().sessions.find((s) => s.id === targetId);
        const hadNoPriorClaude =
          !!target && target.claudeSessionId === e.sessionId && !target.suggestedName; // not yet suggested
        if (hadNoPriorClaude) {
          const label = deriveSessionLabel(e.prompt);
          if (label && label !== target.name) {
            useTerminalsStore.getState().suggestName(targetId, label);
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
  // The Stop hook (both engines) gives a REAL turn-completed signal — flip the
  // status pill to idle immediately instead of waiting out the decay window,
  // and let the user know if they're in another window.
  offs.push(
    await listen<{ termId?: string }>("hook://turn_end", (e) => {
      useAgentStatusStore.getState().markIdle();
      if (!windowIsFocused()) {
        const termId = e.payload?.termId;
        const name = termId
          ? useTerminalsStore.getState().sessions.find((t) => t.id === termId)?.name
          : undefined;
        notify(
          "Agent Console — turn finished",
          name ? `${name} is ready for you` : "The agent finished its turn",
        );
      }
    }),
  );
  return () => {
    for (const off of offs) off();
  };
}
