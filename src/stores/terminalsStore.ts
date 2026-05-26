import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import type { PersistedSession } from "../types/domain";

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
  /// Claude Code session id captured from the UserPromptSubmit hook. When set,
  /// resuming this terminal auto-runs `claude --resume <id>` after spawn.
  claudeSessionId?: string;
}

interface TerminalsState {
  projectRoot: string | null;
  sessions: TerminalSession[];
  activeId: string | null;

  hydrate: (projectRoot: string) => Promise<void>;
  clear: () => void;
  /// Adds a new session (status "live"). Caller is responsible for spawning the PTY.
  add: (cwd: string, name?: string) => string;
  /// Marks a stopped session as live again so Terminal mounts and spawns.
  resume: (id: string) => void;
  setActive: (id: string) => void;
  rename: (id: string, name: string) => void;
  /// Mark session live (e.g., after spawn succeeded).
  markLive: (id: string) => void;
  /// Tag a terminal session with the Claude Code session id from the hook.
  setClaudeSessionId: (id: string, claudeId: string) => void;
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

  hydrate: async (projectRoot) => {
    let persisted: PersistedSession[] = [];
    try {
      persisted = await ipc.sessionsList(projectRoot);
    } catch {
      persisted = [];
    }
    const sessions: TerminalSession[] = persisted.map((p) => ({
      id: p.id,
      name: p.name,
      cwd: p.cwd,
      createdAtMs: p.createdAtMs,
      initialScrollback: p.scrollback,
      liveScrollback: "",
      status: "stopped",
      claudeSessionId: p.claudeSessionId,
    }));
    set({ projectRoot, sessions, activeId: null });
  },

  clear: () => {
    set({ projectRoot: null, sessions: [], activeId: null });
  },

  add: (cwd, name) => {
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
      sessions: sessions.map((s) => (s.id === id ? { ...s, name } : s)),
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
    const { projectRoot, sessions } = get();
    if (!projectRoot) return;
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
        claudeSessionId: s.claudeSessionId,
      };
    });
    try {
      await ipc.sessionsSave(projectRoot, payload);
    } catch {
      /* best-effort */
    }
  },
}));
