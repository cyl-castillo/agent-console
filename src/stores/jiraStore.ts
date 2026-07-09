import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { JiraIssue, JiraStatus } from "../types/domain";

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

  loadStatus: () => Promise<void>;
  connect: (siteUrl: string, email: string, token: string) => Promise<boolean>;
  disconnect: () => Promise<void>;
  refreshIssues: () => Promise<void>;
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

  refreshIssues: async () => {
    set({ loadingIssues: true, issuesError: null });
    try {
      const issues = await ipc.jiraListIssues();
      set({ issues, loadingIssues: false });
    } catch (e) {
      set({ loadingIssues: false, issuesError: String(e) });
    }
  },
}));
