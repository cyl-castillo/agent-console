import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import { useChangesStore } from "./changesStore";
import { usePreviewStore } from "./previewStore";
import { useSessionStore } from "./sessionStore";
import { useSkillsStore } from "./skillsStore";
import { useTerminalsStore } from "./terminalsStore";
import { useThemeStore } from "./themeStore";
import { useToastStore } from "./toastStore";
import { useUpdaterStore } from "./updaterStore";

export type PaletteItemKind = "file" | "action" | "session" | "branch";

export interface PaletteItem {
  id: string;
  kind: PaletteItemKind;
  label: string;
  hint?: string;
  badge?: string;
  warn?: string;
  score: number;
  run: () => void | Promise<void>;
}

export interface PaletteAction {
  id: string;
  label: string;
  hint?: string;
  keywords?: string[];
  available?: () => boolean;
  run: () => void | Promise<void>;
}

interface PaletteState {
  open: boolean;
  query: string;
  selectedIndex: number;
  files: string[];
  filesProjectRoot: string | null;
  filesLoading: boolean;
  filesError: string | null;
  pendingBranchSwitch: string | null;

  openPalette: () => void;
  close: () => void;
  setQuery: (q: string) => void;
  setSelectedIndex: (i: number) => void;
  reloadIndex: () => Promise<void>;
  ensureIndex: (projectRoot: string) => Promise<void>;
  resetForProject: (projectRoot: string | null) => void;
  results: () => PaletteItem[];
  execute: (item: PaletteItem) => Promise<void>;
}

const ACTIONS: PaletteAction[] = [
  {
    id: "nav.terminal",
    label: "Open Terminal",
    hint: "Switch to the Terminal tab",
    keywords: ["shell", "console", "bash"],
    run: () => emit("ac:open-tab", "terminal"),
  },
  {
    id: "nav.changes",
    label: "Open Changes",
    hint: "Switch to the Changes tab",
    keywords: ["git", "diff"],
    run: () => emit("ac:open-tab", "changes"),
  },
  {
    id: "nav.preview",
    label: "Open Preview",
    hint: "Switch to the Preview tab",
    keywords: ["file"],
    run: () => emit("ac:open-tab", "preview"),
  },
  {
    id: "nav.skills",
    label: "Open Skills",
    hint: "Workbench → Skills",
    keywords: ["skills", "prompts"],
    run: () => emit("ac:open-workbench-tab", "skills"),
  },
  {
    id: "nav.permissions",
    label: "Open Permissions",
    hint: "Workbench → Permissions",
    keywords: ["perms", "rules", "allow", "deny"],
    run: () => emit("ac:open-workbench-tab", "permissions"),
  },
  {
    id: "nav.advisor",
    label: "Open Advisor",
    hint: "Workbench → Advisor",
    keywords: ["recommend", "analyze"],
    run: () => emit("ac:open-workbench-tab", "advisor"),
  },
  {
    id: "nav.vault",
    label: "Open Vault",
    hint: "Workbench → Vault",
    keywords: ["secrets", "private"],
    run: () => emit("ac:open-workbench-tab", "vault"),
  },
  {
    id: "nav.context",
    label: "Open Context (CLAUDE.md & memories)",
    hint: "Workbench → Context",
    keywords: ["claude", "md", "memory"],
    run: () => emit("ac:open-workbench-tab", "context"),
  },
  {
    id: "nav.plugins",
    label: "Open Plugins",
    hint: "Workbench → Plugins",
    keywords: ["plugin", "marketplace"],
    run: () => emit("ac:open-workbench-tab", "plugins"),
  },
  {
    id: "nav.mcp",
    label: "Open MCP Servers",
    hint: "Workbench → MCP",
    keywords: ["server", "connector", "tools"],
    run: () => emit("ac:open-workbench-tab", "mcp"),
  },
  {
    id: "nav.feedback",
    label: "Open Feedback",
    hint: "Workbench → Feedback",
    keywords: ["dev", "report"],
    run: () => emit("ac:open-workbench-tab", "feedback"),
  },
  {
    id: "git.commit",
    label: "Open Git Commit",
    hint: "Changes tab and focus the commit message",
    keywords: ["git", "message"],
    run: () => {
      emit("ac:open-tab", "changes");
      setTimeout(() => emit("ac:focus-commit", null), 50);
    },
  },
  {
    id: "git.refresh",
    label: "Refresh Git Status",
    hint: "Re-runs git status (safe)",
    keywords: ["reload", "sync"],
    run: () => { void useChangesStore.getState().refresh(); },
  },
  {
    id: "session.new",
    label: "New Session",
    hint: "Start a new live agent terminal",
    keywords: ["terminal", "shell", "agent"],
    run: () => emit("ac:new-session", null),
  },
  {
    id: "session.close",
    label: "Close Active Session",
    hint: "Stops and removes the active terminal session",
    keywords: ["terminal", "kill", "remove"],
    available: () => !!useTerminalsStore.getState().activeId,
    run: async () => {
      const st = useTerminalsStore.getState();
      const active = st.sessions.find((s) => s.id === st.activeId);
      if (!active) return;
      if (active.status === "live" && !confirm(`Close session "${active.name}"? Process will be killed.`)) return;
      await st.close(active.id);
    },
  },
  {
    id: "ui.toggle_sidebar",
    label: "Toggle Workspace Sidebar",
    hint: "Show or hide Sessions, Changes, and Files",
    keywords: ["left", "workspace"],
    run: () => emit("ac:toggle-sidebar", null),
  },
  {
    id: "ui.toggle_theme",
    label: "Toggle Theme",
    hint: "Switch between dark and light themes",
    keywords: ["dark", "light"],
    run: () => useThemeStore.getState().toggle(),
  },
  {
    id: "project.copy_path",
    label: "Copy Project Path",
    hint: "Copy the current project root to clipboard",
    keywords: ["cwd", "folder", "root"],
    available: () => !!useSessionStore.getState().project,
    run: () => emit("ac:copy-project-path", null),
  },
  {
    id: "app.check_updates",
    label: "Check for Updates",
    hint: "Run the in-app update check",
    keywords: ["version", "upgrade"],
    run: async () => {
      await useUpdaterStore.getState().check({ silentIfNone: false });
      const phase = useUpdaterStore.getState().phase;
      if (phase === "uptodate") useToastStore.getState().show("Agent Console is up to date", "success");
    },
  },
  {
    id: "snapshot.restore_latest",
    label: "Restore Latest Snapshot",
    hint: "Restore to before the latest captured turn",
    keywords: ["undo", "rollback", "restore"],
    available: () => useSkillsStore.getState().recent.some((e) => !!e.snapshotCommitSha),
    run: async () => {
      const event = useSkillsStore.getState().recent.find((e) => !!e.snapshotCommitSha);
      if (!event?.snapshotCommitSha) return;
      if (!confirm("Restore to before the latest turn? Uncommitted changes will be lost.")) return;
      await useSkillsStore.getState().restoreSnapshot(event.snapshotCommitSha);
      useToastStore.getState().show("Snapshot restored", "success");
    },
  },
  {
    id: "help.getting_started",
    label: "Open Getting Started",
    hint: "Onboarding checklist",
    keywords: ["help", "guide", "tutorial"],
    run: () => emit("ac:open-getting-started", null),
  },
];

function emit(name: string, detail: unknown) {
  window.dispatchEvent(new CustomEvent(name, { detail }));
}

/** Subsequence fuzzy score. Higher is better. Returns -1 if not a match. */
function fuzzyScore(query: string, target: string): number {
  if (!query) return 1;
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  if (q === t) return 1_000_000;
  const direct = t.indexOf(q);
  if (direct !== -1) return 100_000 - direct;
  // subsequence
  let ti = 0;
  let score = 0;
  let lastMatch = -2;
  let streak = 0;
  for (let qi = 0; qi < q.length; qi++) {
    const ch = q[qi];
    let found = -1;
    while (ti < t.length) {
      if (t[ti] === ch) { found = ti; ti++; break; }
      ti++;
    }
    if (found === -1) return -1;
    if (found === lastMatch + 1) { streak++; score += 5 + streak * 2; }
    else { streak = 0; score += 1; }
    // boost on word boundaries
    if (found === 0 || /[\/_\-. ]/.test(t[found - 1])) score += 8;
    lastMatch = found;
  }
  // shorter targets win ties
  score += Math.max(0, 40 - t.length);
  return score;
}

function scoreFile(query: string, path: string): number {
  const base = path.split("/").pop() ?? path;
  const baseScore = fuzzyScore(query, base);
  const pathScore = fuzzyScore(query, path);
  if (baseScore === -1 && pathScore === -1) return -1;
  return Math.max(baseScore * 2, pathScore);
}

export const usePaletteStore = create<PaletteState>((set, get) => ({
  open: false,
  query: "",
  selectedIndex: 0,
  files: [],
  filesProjectRoot: null,
  filesLoading: false,
  filesError: null,
  pendingBranchSwitch: null,

  openPalette: () => {
    set({ open: true, query: "", selectedIndex: 0, pendingBranchSwitch: null });
    // Build the file index lazily, on first palette open, instead of eagerly on
    // project open. The index is a full-tree walk; doing it here keeps it off
    // the project-open critical path (a real win on Windows, where every
    // stat/readdir is intercepted by Defender). The palette shows an
    // "Indexing files…" state while it loads.
    const root = useSessionStore.getState().project?.root;
    if (root) void get().ensureIndex(root);
    // Lazy-load branches if not yet loaded (cheap, sync UI).
    const cs = useChangesStore.getState();
    if (cs.branches.length === 0 && !cs.branchesLoading) {
      void cs.loadBranches();
    }
  },
  close: () => set({ open: false, query: "", selectedIndex: 0, pendingBranchSwitch: null }),
  setQuery: (q) => set({ query: q, selectedIndex: 0, pendingBranchSwitch: null }),
  setSelectedIndex: (i) => set({ selectedIndex: i, pendingBranchSwitch: null }),

  ensureIndex: async (projectRoot) => {
    if (get().filesProjectRoot === projectRoot && get().files.length > 0) return;
    await get().reloadIndex();
  },

  reloadIndex: async () => {
    set({ filesLoading: true, filesError: null });
    try {
      const files = await ipc.paletteIndexFiles();
      const root = useSessionStore.getState().project?.root ?? null;
      set({ files, filesProjectRoot: root, filesLoading: false });
    } catch (e) {
      set({ filesLoading: false, filesError: String(e) });
    }
  },

  resetForProject: (projectRoot) => {
    set({
      files: [],
      filesProjectRoot: projectRoot,
      filesLoading: false,
      filesError: null,
      open: false,
      query: "",
      selectedIndex: 0,
      pendingBranchSwitch: null,
    });
  },

  results: () => {
    const { query, files } = get();
    const raw = query.trim();
    let mode: "all" | "action" | "session" | "branch" = "all";
    let q = raw;
    if (raw.startsWith(">")) { mode = "action"; q = raw.slice(1).trim(); }
    else if (raw.startsWith(":")) { mode = "session"; q = raw.slice(1).trim(); }
    else if (raw.startsWith("@")) { mode = "branch"; q = raw.slice(1).trim(); }

    const items: PaletteItem[] = [];

    if (mode === "all" || mode === "action") {
      for (const a of ACTIONS) {
        if (a.available && !a.available()) continue;
        const haystack = [a.label, ...(a.keywords ?? [])].join(" ");
        const s = q ? fuzzyScore(q, haystack) : 50;
        if (s < 0) continue;
        items.push({
          id: `action:${a.id}`,
          kind: "action",
          label: a.label,
          hint: a.hint,
          score: s + 10, // small boost so actions float up alongside files
          run: a.run,
        });
      }
    }

    if (mode === "all") {
      const limit = 200;
      const scored: { p: string; s: number }[] = [];
      if (!q) {
        // empty query: nothing to rank — show first N alphabetical-ish
        for (let i = 0; i < Math.min(files.length, 30); i++) {
          scored.push({ p: files[i], s: 10 });
        }
      } else {
        for (let i = 0; i < files.length; i++) {
          const s = scoreFile(q, files[i]);
          if (s >= 0) scored.push({ p: files[i], s });
          if (scored.length > limit * 4) break;
        }
      }
      scored.sort((a, b) => b.s - a.s);
      for (let i = 0; i < Math.min(scored.length, 30); i++) {
        const path = scored[i].p;
        const base = path.split("/").pop() ?? path;
        const dir = path.length > base.length ? path.slice(0, path.length - base.length - 1) : "";
        items.push({
          id: `file:${path}`,
          kind: "file",
          label: base,
          hint: dir || undefined,
          score: scored[i].s,
          run: async () => {
            const root = useSessionStore.getState().project?.root;
            if (!root) return;
            const abs = `${root}/${path}`;
            emit("ac:open-tab", "preview");
            await usePreviewStore.getState().open(abs);
          },
        });
      }
    }

    if (mode === "session" || (mode === "all" && raw === "")) {
      const sessions = useTerminalsStore.getState().sessions;
      const activeId = useTerminalsStore.getState().activeId;
      for (const s of sessions) {
        const haystack = `${s.name} ${s.cwd}`;
        const sc = q ? fuzzyScore(q, haystack) : 5;
        if (sc < 0) continue;
        items.push({
          id: `session:${s.id}`,
          kind: "session",
          label: s.name,
          hint: s.id === activeId ? "active" : s.status,
          score: sc,
          run: () => {
            useTerminalsStore.getState().setActive(s.id);
            emit("ac:open-tab", "terminal");
          },
        });
      }
    }

    if (mode === "branch") {
      const cs = useChangesStore.getState();
      const branches = cs.branches;
      const dirty = (cs.status?.changes.length ?? 0) > 0;
      const current = cs.status?.branch ?? null;
      for (const b of branches) {
        const sc = q ? fuzzyScore(q, b.name) : 5;
        if (sc < 0) continue;
        const isCurrent = b.name === current;
        items.push({
          id: `branch:${b.name}`,
          kind: "branch",
          label: b.name,
          hint: isCurrent ? "current" : undefined,
          badge: dirty && !isCurrent ? "⚠ uncommitted changes" : undefined,
          warn: dirty && !isCurrent ? "Press Enter again to confirm switch" : undefined,
          score: sc + (isCurrent ? -10 : 0),
          run: async () => {
            if (isCurrent) return;
            await cs.checkoutBranch(b.name);
          },
        });
      }
    }

    items.sort((a, b) => b.score - a.score);
    return items.slice(0, 60);
  },

  execute: async (item) => {
    // Branch safety: if there are uncommitted changes and item is a branch
    // switch to a different branch, require a second Enter to confirm.
    if (item.kind === "branch" && item.warn) {
      const { pendingBranchSwitch } = get();
      if (pendingBranchSwitch !== item.id) {
        set({ pendingBranchSwitch: item.id });
        return;
      }
    }
    try {
      await item.run();
    } finally {
      set({ open: false, query: "", selectedIndex: 0, pendingBranchSwitch: null });
    }
  },
}));
