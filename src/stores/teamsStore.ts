import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { TeamsChat, TeamsDeviceLogin, TeamsMessage, TeamsStatus } from "../types/domain";

/// Read-only Teams companion: device-code login, your chats, their messages.
/// The store owns the login POLL LOOP (the panel only renders its state) so
/// closing/reopening the panel mid-login doesn't orphan or duplicate it.
interface TeamsState {
  status: TeamsStatus | null;
  loadingStatus: boolean;
  loadStatus: () => Promise<void>;

  /// Non-null while the device-code screen should be shown.
  login: TeamsDeviceLogin | null;
  connecting: boolean;
  connectError: string | null;
  beginLogin: (clientId: string, tenant: string) => Promise<void>;
  cancelLogin: () => Promise<void>;
  disconnect: () => Promise<void>;

  chats: TeamsChat[];
  loadingChats: boolean;
  chatsError: string | null;
  refreshChats: () => Promise<void>;

  /// Chat currently open in the messages view; null = chat list.
  activeChatId: string | null;
  messages: TeamsMessage[];
  loadingMessages: boolean;
  messagesError: string | null;
  openChat: (chatId: string) => Promise<void>;
  closeChat: () => void;
}

const DISCONNECTED: TeamsStatus = { configured: false, clientId: "", tenant: "", account: "" };

export const useTeamsStore = create<TeamsState>((set, get) => {
  // Poll-loop bookkeeping lives in the closure, not in state: bumping the
  // generation orphans any in-flight loop (cancel, disconnect, re-begin).
  let pollGen = 0;
  let deadlineMs = 0;

  async function pollLoop(gen: number, intervalMs: number) {
    await new Promise((r) => setTimeout(r, intervalMs));
    if (gen !== pollGen) return;
    if (Date.now() > deadlineMs) {
      set({
        connecting: false,
        login: null,
        connectError: "The code expired before the sign-in finished. Connect again.",
      });
      void ipc.teamsCancelLogin().catch(() => {});
      return;
    }
    try {
      const poll = await ipc.teamsPollLogin();
      if (gen !== pollGen) return;
      if (poll.state === "pending") {
        void pollLoop(gen, intervalMs);
      } else if (poll.state === "connected") {
        set({ connecting: false, login: null, connectError: null });
        await get().loadStatus();
      } else {
        set({ connecting: false, login: null, connectError: poll.message });
      }
    } catch (e) {
      if (gen !== pollGen) return;
      set({ connecting: false, login: null, connectError: String(e) });
    }
  }

  return {
    status: null,
    loadingStatus: false,
    loadStatus: async () => {
      set({ loadingStatus: true });
      try {
        const status = await ipc.teamsStatus();
        set({ status, loadingStatus: false });
        if (status.configured) void get().refreshChats();
      } catch {
        set({ loadingStatus: false, status: DISCONNECTED });
      }
    },

    login: null,
    connecting: false,
    connectError: null,
    beginLogin: async (clientId, tenant) => {
      if (get().connecting) return;
      set({ connecting: true, connectError: null });
      try {
        const login = await ipc.teamsBeginLogin(clientId, tenant);
        deadlineMs = Date.now() + login.expiresInSecs * 1000;
        set({ login });
        void pollLoop(++pollGen, Math.max(login.intervalMs, 1000));
      } catch (e) {
        set({ connecting: false, connectError: String(e) });
      }
    },
    cancelLogin: async () => {
      pollGen++;
      set({ connecting: false, login: null });
      try {
        await ipc.teamsCancelLogin();
      } catch {
        /* best-effort */
      }
    },
    disconnect: async () => {
      pollGen++;
      try {
        await ipc.teamsDisconnect();
      } catch {
        /* best-effort */
      }
      set({
        status: DISCONNECTED,
        login: null,
        connecting: false,
        connectError: null,
        chats: [],
        chatsError: null,
        activeChatId: null,
        messages: [],
        messagesError: null,
      });
    },

    chats: [],
    loadingChats: false,
    chatsError: null,
    refreshChats: async () => {
      set({ loadingChats: true, chatsError: null });
      try {
        const chats = await ipc.teamsListChats();
        set({ chats, loadingChats: false });
      } catch (e) {
        set({ loadingChats: false, chatsError: String(e) });
      }
    },

    activeChatId: null,
    messages: [],
    loadingMessages: false,
    messagesError: null,
    openChat: async (chatId) => {
      set({ activeChatId: chatId, messages: [], loadingMessages: true, messagesError: null });
      try {
        const messages = await ipc.teamsListMessages(chatId);
        // A slow response for a chat the user already left must not clobber
        // the currently-open one.
        if (get().activeChatId === chatId) set({ messages, loadingMessages: false });
      } catch (e) {
        if (get().activeChatId === chatId)
          set({ loadingMessages: false, messagesError: String(e) });
      }
    },
    closeChat: () => set({ activeChatId: null, messages: [], messagesError: null }),
  };
});
