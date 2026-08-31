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
import { reconcileSwitchedModel } from "../agents/profiles";

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
export function deriveSessionLabel(prompt: string): string {
  let s = prompt
    .replace(/```[\s\S]*?```/g, " ") // drop fenced code blocks
    .replace(/`[^`]*`/g, " ") // drop inline code
    .replace(/https?:\/\/\S+/g, " ") // drop urls
    .replace(/[#*_>`~|]/g, " ") // drop markdown punctuation
    .replace(/[\r\n]+/g, " ")
    .trim();
  // Strip leading filler FIRST — greetings often carry the sentence
  // punctuation ("hola! …"), and cutting at that boundary first would leave
  // nothing ("dale, arreglá el login" → "arreglá el login").
  const FILLER = new Set([
    "hola",
    "hey",
    "hi",
    "hello",
    "buenas",
    "dale",
    "listo",
    "ok",
    "okay",
    "bueno",
    "porfa",
    "please",
    "pls",
    "seguimos",
    "vamos",
    "ahora",
    "si",
    "sí",
    "no",
    "y",
    "e",
    "a",
    "ver",
  ]);
  let words = s.split(/\s+/).filter(Boolean);
  while (words.length > 0 && FILLER.has(words[0].toLowerCase().replace(/[,.:;!¡¿?]+$/, "")))
    words.shift();
  s = words.join(" ");
  // Then cut at the first sentence boundary if present.
  const m = s.match(/^[^.!?\n]{3,}/);
  if (m) s = m[0];
  words = s.split(/\s+/).filter(Boolean);
  // A name needs substance: a vague or greeting-only prompt names nothing —
  // better to stay "shell N" than to wear a junk label.
  const meaningful = words.slice(0, 5);
  let label = meaningful
    .join(" ")
    .slice(0, 40)
    .trim()
    .replace(/[,.:;]+$/, "");
  if (meaningful.length < 3 || label.length < 12) return "";
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
    // First prompt EVER on this machine: the event arriving here is live proof
    // the hook bridges work end-to-end — exactly what the first-run wizard
    // promised the console would confirm. Say it once.
    if (!useOnboardingStore.getState().promptedClaude) {
      useToastStore.getState().show("First prompt captured — the console's bridges work ✓", "info");
    }
    useOnboardingStore.getState().markPromptedClaude();
    // A submitted prompt means a turn just started — light the "working" pill.
    useAgentStatusStore.getState().markActive();
    // Feed the learning auto-trigger: enough new activity reflects on its own.
    useLearningStore.getState().noteActivity();
    // Notify scheduler jobs watching for prompts.
    void fireSchedulerEvent("prompt");

    // Associate the agent's session id with the terminal that emitted the prompt.
    // The hook tags each prompt with the PTY's terminal-session id (termId, from
    // AGENT_CONSOLE_TERM_ID), so we can bind deterministically — even when several
    // claude sessions run at once. We only fall back to the active terminal when
    // termId is missing (e.g. a claude launched before this build's hook change).
    if (e.sessionId) {
      const { activeId, sessions } = useTerminalsStore.getState();
      const targetId = e.termId && sessions.some((s) => s.id === e.termId) ? e.termId : activeId;
      if (targetId) {
        useTerminalsStore.getState().setAgentSessionId(targetId, e.sessionId);
        // Silently name the session from its first prompt — autoName only ever
        // replaces a default "shell N" name, never a user- or ticket-chosen
        // one, and only once (nameSuggested marker).
        const label = deriveSessionLabel(e.prompt);
        if (label) useTerminalsStore.getState().autoName(targetId, label);
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
  // PostModelSwitch (Claude 2.1.251+) is the first signal that tells us what the
  // agent is REALLY running. Until now the model pill could only show intent —
  // the last value we asked for — and the two drift apart on a `/model` typed in
  // the terminal, on our own `/model` push landing while the agent was busy, and
  // on Claude restoring or falling back to another model by itself. The drift
  // isn't just cosmetic: the stale value is what `--model` pins on resume, so a
  // wrong pill actively drags the session back to the wrong model.
  offs.push(
    await listen<{ termId?: string; model?: string }>("hook://model_switch", (e) => {
      const { termId, model } = e.payload ?? {};
      if (!termId || !model) return;
      const session = useTerminalsStore.getState().sessions.find((s) => s.id === termId);
      // No termId match ⇒ drop it. Unlike the prompt hook we do NOT fall back to
      // the active terminal: writing another session's model onto whatever is
      // focused would be worse than knowing nothing.
      if (!session) return;
      const next = reconcileSwitchedModel(session.agent, session.model, model);
      if (next !== null) useTerminalsStore.getState().setModel(termId, next);
    }),
  );
  return () => {
    for (const off of offs) off();
  };
}
