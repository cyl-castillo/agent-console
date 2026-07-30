import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { JiraIssue, JiraStatus, WorklogDigestEntry } from "../types/domain";
import { useSessionStore } from "./sessionStore";

/// Local-midnight bounds + YYYY-MM-DD for "today minus offsetDays".
export function localDay(offsetDays: number): { startMs: number; endMs: number; date: string } {
  const now = new Date();
  const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - offsetDays);
  const startMs = d.getTime();
  const p = (n: number) => String(n).padStart(2, "0");
  return {
    startMs,
    endMs: startMs + 86_400_000,
    date: `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`,
  };
}

interface JiraState {
  status: JiraStatus | null;
  issues: JiraIssue[];
  loadingStatus: boolean;
  loadingIssues: boolean;
  connecting: boolean;
  /// Error from the last connect attempt (shown in the form).
  connectError: string | null;
  /// Error from the last issue fetch (shown above the list).
  issuesError: string | null;

  /// Effective JQL for the Queue (role preset or hand-tuned). null = backend
  /// default ("assigned to me, not Done"). Changing it triggers a refresh.
  jql: string | null;
  setJql: (jql: string | null) => Promise<void>;
  loadStatus: () => Promise<void>;
  connect: (siteUrl: string, email: string, token: string) => Promise<boolean>;
  disconnect: () => Promise<void>;
  refreshIssues: () => Promise<void>;
  /// Log time on an issue. Resolves to the normalized label logged ("1h 30m")
  /// or null on failure (error surfaced via the returned message in `logError`).
  logWork: (
    issueKey: string,
    duration: string,
    started: string,
    comment?: string,
  ) => Promise<string | null>;
  logError: string | null;

  /// The "⏱ Today / Yesterday" digest: witnessed time per ticket for a local
  /// day, prefilled and human-reviewed before anything reaches Jira.
  digestDate: string | null;
  digest: WorklogDigestEntry[];
  loadingDigest: boolean;
  loadDigest: (offsetDays: number) => Promise<void>;
  /// Log the given entries for the loaded day. Returns [ok, failed] counts.
  logDay: (entries: { issueKey: string; duration: string }[]) => Promise<[number, number]>;
}

export const useJiraStore = create<JiraState>((set, get) => ({
  status: null,
  issues: [],
  loadingStatus: false,
  loadingIssues: false,
  connecting: false,
  connectError: null,
  issuesError: null,

  loadStatus: async () => {
    set({ loadingStatus: true });
    try {
      const status = await ipc.jiraStatus();
      set({ status, loadingStatus: false });
      if (status.configured) void get().refreshIssues();
    } catch (e) {
      set({ loadingStatus: false, status: { configured: false, siteUrl: "", email: "" } });
      void e;
    }
  },

  connect: async (siteUrl, email, token) => {
    if (get().connecting) return false;
    set({ connecting: true, connectError: null });
    try {
      await ipc.jiraConnect(siteUrl, email, token);
      set({ connecting: false });
      await get().loadStatus();
      return true;
    } catch (e) {
      set({ connecting: false, connectError: String(e) });
      return false;
    }
  },

  disconnect: async () => {
    try {
      await ipc.jiraDisconnect();
    } catch {
      /* best-effort */
    }
    set({
      status: { configured: false, siteUrl: "", email: "" },
      issues: [],
      issuesError: null,
      connectError: null,
    });
  },

  jql: null,
  setJql: async (jql) => {
    if (get().jql === jql) return;
    set({ jql });
    if (get().status?.configured) await get().refreshIssues();
  },

  refreshIssues: async () => {
    set({ loadingIssues: true, issuesError: null });
    try {
      const issues = await ipc.jiraListIssues(get().jql ?? undefined);
      set({ issues, loadingIssues: false });
    } catch (e) {
      set({ loadingIssues: false, issuesError: String(e) });
    }
  },

  digestDate: null,
  digest: [],
  loadingDigest: false,
  loadDigest: async (offsetDays) => {
    const root = useSessionStore.getState().project?.root;
    if (!root) return;
    const day = localDay(offsetDays);
    set({ loadingDigest: true, digestDate: day.date });
    try {
      const digest = await ipc.jiraDailyDigest(root, day.startMs, day.endMs, day.date);
      // A slow response for a day the user already navigated away from must
      // not clobber the currently-shown one.
      if (get().digestDate === day.date) set({ digest, loadingDigest: false });
    } catch {
      if (get().digestDate === day.date) set({ digest: [], loadingDigest: false });
    }
  },
  logDay: async (entries) => {
    const root = useSessionStore.getState().project?.root;
    const date = get().digestDate;
    if (!root || !date || entries.length === 0) return [0, 0];
    try {
      const results = await ipc.jiraLogDay(root, date, entries);
      const ok = results.filter((r) => r.ok).length;
      // Refresh marks state for the same day.
      const day = localDay(0);
      const offset = date === day.date ? 0 : 1;
      await get().loadDigest(offset);
      return [ok, results.length - ok];
    } catch (e) {
      set({ logError: String(e) });
      return [0, entries.length];
    }
  },

  logError: null,
  logWork: async (issueKey, duration, started, comment) => {
    set({ logError: null });
    try {
      return await ipc.jiraLogWork(issueKey, duration, started, comment);
    } catch (e) {
      set({ logError: String(e) });
      return null;
    }
  },
}));
