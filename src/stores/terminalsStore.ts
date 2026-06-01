import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import type { PersistedSession } from "../types/domain";
import { asAgentKind, type AgentKind } from "../agents/profiles";

const MAX_SCROLLBACK = 50_000;

export interface TerminalSession {
  id: string;
  name: string;
  cwd: string;
  createdAtMs: number;
  /// Saved output from a previous run, replayed when the user resumes.
  initialScrollback: string;
  /// Live scrollback captured while running. Updated by Terminal via setScrollback.
  liveScrollback: string;
  /// "stopped" = not spawned yet (from disk); "live" = PTY currently running.
  status: "live" | "stopped";
  /// Which coding agent this terminal launches. Undefined = Claude (the default
  /// and the only option for sessions created before agent selection existed).
  agent?: AgentKind;
  /// Claude Code session id captured from the UserPromptSubmit hook. When set,
  /// resuming this terminal auto-runs `claude --resume <id>` after spawn.
  /// Codex sessions never populate this (no hook to capture it).
  claudeSessionId?: string;
  /// Derived from the first user prompt; offered inline as a rename suggestion.
  /// Cleared once accepted, dismissed, or when the user renames manually.
  suggestedName?: string;
  /// True after the very first auto-suggestion fires for this session. Keeps
  /// us from re-suggesting on every subsequent prompt.
  nameSuggested?: boolean;
  /// Model alias or full id chosen for this session. Passed as
  /// `claude --model <model>` on launch; undefined = account default.
  model?: string;
}

interface TerminalsState {
  projectRoot: string | null;
  sessions: TerminalSession[];
  activeId: string | null;
  /// True only after a successful hydrate for the current project. While false
  /// (initial state, or after a failed load) persist() is blocked so a failed
  /// read can never overwrite the saved history with an empty/partial list.
  ready: boolean;

  hydrate: (projectRoot: string) => Promise<void>;
  clear: () => void;
  /// Adds a new session (status "live"). Caller is responsible for spawning the PTY.
  add: (cwd: string, name?: string, model?: string, agent?: AgentKind) => string;
  /// Marks a stopped session as live again so Terminal mounts and spawns.
  resume: (id: string) => void;
  setActive: (id: string) => void;
  rename: (id: string, name: string) => void;
  /// Mark session live (e.g., after spawn succeeded).
  markLive: (id: string) => void;
  /// Tag a terminal session with the Claude Code session id from the hook.
  setClaudeSessionId: (id: string, claudeId: string) => void;
  /// Set (or clear, with undefined) the model for a session and persist it.
  setModel: (id: string, model: string | undefined) => void;
  suggestName: (id: string, name: string) => void;
  acceptSuggestion: (id: string) => void;
  dismissSuggestion: (id: string) => void;
  /// Buffer output bytes into the live scrollback (called frequently — does not notify subscribers).
  appendOutput: (id: string, chunk: string) => void;
  /// Remove session entirely (kill+delete).
  close: (id: string) => Promise<void>;
  /// Persist current session list (metadata + scrollback) for this project.
  persist: () => Promise<void>;
}

function genId(): string {
  return `t-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function nextName(existing: TerminalSession[]): string {
  const used = new Set(existing.map((s) => s.name));
  for (let i = 1; i < 100; i++) {
    const n = `shell ${i}`;
    if (!used.has(n)) return n;
  }
  return `shell ${Date.now()}`;
}

export const useTerminalsStore = create<TerminalsState>((set, get) => ({
  projectRoot: null,
  sessions: [],
  activeId: null,
  ready: false,

  hydrate: async (projectRoot) => {
    let persisted: PersistedSession[];
    try {
      persisted = await ipc.sessionsList(projectRoot);
    } catch (e) {
      // Read failed (corrupt/unreadable file). Do NOT touch the saved history:
      // leave ready=false so persist() is blocked and the app won't auto-spawn
      // a session that would otherwise overwrite the file on the next save.
      console.error("[sessions] hydrate failed; not overwriting saved history:", e);
      set({ projectRoot, sessions: [], activeId: null, ready: false });
      return;
    }
    const sessions: TerminalSession[] = persisted.map((p) => ({
      id: p.id,
      name: p.name,
      cwd: p.cwd,
      createdAtMs: p.createdAtMs,
      nameSuggested: p.nameSuggested,
      initialScrollback: p.scrollback,
      liveScrollback: "",
      status: "stopped",
      agent: asAgentKind(p.agent),
      claudeSessionId: p.claudeSessionId,
      model: p.model,
    }));
    set({ projectRoot, sessions, activeId: null, ready: true });
  },

  clear: () => {
    set({ projectRoot: null, sessions: [], activeId: null, ready: false });
  },

  add: (cwd, name, model, agent) => {
    const id = genId();
    const { sessions } = get();
    const session: TerminalSession = {
      id,
      name: name ?? nextName(sessions),
      cwd,
      createdAtMs: Date.now(),
      initialScrollback: "",
      liveScrollback: "",
      status: "live",
      agent,
      model,
    };
    set({ sessions: [...sessions, session], activeId: id });
    return id;
  },

  resume: (id) => {
    const { sessions } = get();
    set({
      sessions: sessions.map((s) =>
        s.id === id ? { ...s, status: "live" as const } : s,
      ),
      activeId: id,
    });
  },

  setActive: (id) => set({ activeId: id }),

  rename: (id, name) => {
    const { sessions } = get();
    set({
      // Manual rename clears any pending suggestion.
      sessions: sessions.map((s) => (s.id === id ? { ...s, name, suggestedName: undefined } : s)),
    });
  },

  suggestName: (id, name) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const { sessions } = get();
    const target = sessions.find((s) => s.id === id);
    if (!target) return;
    if (target.nameSuggested) return;               // already had its shot
    if (target.name === trimmed) return;            // already named that
    if (target.suggestedName === trimmed) return;   // same suggestion already pending
    set({
      sessions: sessions.map((s) => (s.id === id ? { ...s, suggestedName: trimmed, nameSuggested: true } : s)),
    });
    get().persist();
  },

  acceptSuggestion: (id) => {
    const { sessions } = get();
    const target = sessions.find((s) => s.id === id);
    if (!target?.suggestedName) return;
    set({
      sessions: sessions.map((s) =>
        s.id === id ? { ...s, name: s.suggestedName ?? s.name, suggestedName: undefined } : s,
      ),
    });
    get().persist();
  },

  dismissSuggestion: (id) => {
    const { sessions } = get();
    set({
      sessions: sessions.map((s) => (s.id === id ? { ...s, suggestedName: undefined } : s)),
    });
  },

  markLive: (id) => {
    const { sessions } = get();
    set({
      sessions: sessions.map((s) =>
        s.id === id ? { ...s, status: "live" as const } : s,
      ),
    });
  },

  setClaudeSessionId: (id, claudeId) => {
    const { sessions } = get();
    const target = sessions.find((s) => s.id === id);
    if (!target || target.claudeSessionId === claudeId) return;
    set({
      sessions: sessions.map((s) =>
        s.id === id ? { ...s, claudeSessionId: claudeId } : s,
      ),
    });
    // Persist immediately so a crash doesn't lose the association.
    get().persist();
  },

  setModel: (id, model) => {
    const { sessions } = get();
    const target = sessions.find((s) => s.id === id);
    if (!target || target.model === model) return;
    set({
      sessions: sessions.map((s) => (s.id === id ? { ...s, model } : s)),
    });
    get().persist();
  },

  // Mutates in place — frequent calls (output streaming) should not trigger React re-renders.
  // We never depend on liveScrollback in selectors; it's only read at persist() time.
  appendOutput: (id, chunk) => {
    const { sessions } = get();
    const idx = sessions.findIndex((s) => s.id === id);
    if (idx < 0) return;
    const target = sessions[idx];
    const next = target.liveScrollback + chunk;
    target.liveScrollback =
      next.length > MAX_SCROLLBACK ? next.slice(next.length - MAX_SCROLLBACK) : next;
  },

  close: async (id) => {
    const { sessions, activeId } = get();
    const remaining = sessions.filter((s) => s.id !== id);
    const nextActive =
      activeId === id ? (remaining[remaining.length - 1]?.id ?? null) : activeId;
    set({ sessions: remaining, activeId: nextActive });
    await get().persist();
  },

  persist: async () => {
    const { projectRoot, sessions, ready } = get();
    // Block until a successful hydrate: persisting while !ready could overwrite
    // saved history that we failed to (or haven't yet) read back.
    if (!projectRoot || !ready) return;
    const payload: PersistedSession[] = sessions.map((s) => {
      const scrollback = s.status === "live" ? s.liveScrollback : s.initialScrollback;
      const trimmed =
        scrollback.length > MAX_SCROLLBACK
          ? scrollback.slice(scrollback.length - MAX_SCROLLBACK)
          : scrollback;
      return {
        id: s.id,
        name: s.name,
        cwd: s.cwd,
        createdAtMs: s.createdAtMs,
        scrollback: trimmed,
        agent: s.agent,
        claudeSessionId: s.claudeSessionId,
        nameSuggested: s.nameSuggested,
        model: s.model,
      };
    });
    try {
      await ipc.sessionsSave(projectRoot, payload);
    } catch {
      /* best-effort */
    }
  },
}));
