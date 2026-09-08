import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useTeamsStore } from "../stores/teamsStore";
import { useToastStore } from "../stores/toastStore";
import { typeIntoActiveSession } from "../lib/termInput";
import { PanelError } from "./PanelError";
import type { TeamsMessage } from "../types/domain";

/// Read-only Microsoft Teams companion: see the messages people send you and
/// pull their text into the composer. Never posts back — there is no send path.
export function TeamsPanel() {
  const status = useTeamsStore((s) => s.status);
  const loadingStatus = useTeamsStore((s) => s.loadingStatus);
  const loadStatus = useTeamsStore((s) => s.loadStatus);
  const login = useTeamsStore((s) => s.login);
  const activeChatId = useTeamsStore((s) => s.activeChatId);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">teams</span>
        {status?.configured && <TeamsHeaderActions />}
      </div>
      <div className="workbench-body">
        {loadingStatus && !status ? (
          <div className="wb-hint">Loading…</div>
        ) : login ? (
          <DeviceCodeScreen />
        ) : status?.configured ? (
          activeChatId ? (
            <MessagesView />
          ) : (
            <ChatList />
          )
        ) : (
          <ConnectForm />
        )}
      </div>
    </div>
  );
}

function TeamsHeaderActions() {
  const account = useTeamsStore((s) => s.status?.account ?? "");
  const refreshChats = useTeamsStore((s) => s.refreshChats);
  const loadingChats = useTeamsStore((s) => s.loadingChats);
  const disconnect = useTeamsStore((s) => s.disconnect);
  return (
    <span className="teams-header-actions">
      {account && <span className="teams-account">{account}</span>}
      <button
        className="workbench-action"
        onClick={() => void refreshChats()}
        disabled={loadingChats}
        title="Refresh chats"
      >
        ↻
      </button>
      <button
        className="workbench-action"
        onClick={() => void disconnect()}
        title="Disconnect Teams (forgets the tokens in your keychain)"
      >
        ⏏
      </button>
    </span>
  );
}

function ConnectForm() {
  const beginLogin = useTeamsStore((s) => s.beginLogin);
  const connecting = useTeamsStore((s) => s.connecting);
  const connectError = useTeamsStore((s) => s.connectError);
  const savedClientId = useTeamsStore((s) => s.status?.clientId ?? "");
  const savedTenant = useTeamsStore((s) => s.status?.tenant ?? "");
  const [clientId, setClientId] = useState(savedClientId);
  const [tenant, setTenant] = useState(savedTenant);

  const submit = () => {
    if (connecting || !clientId.trim()) return;
    void beginLogin(clientId.trim(), tenant.trim());
  };

  return (
    <div className="jira-connect">
      <p className="wb-hint wb-trust">
        Read-only: this panel can see your own chats, never post to them. You sign in with your
        Microsoft account (device code — no password touches this app) and the tokens live in your
        OS keychain, never in a file or log.
      </p>
      <p className="wb-hint">
        You bring your own Entra app registration: multi-tenant, “Allow public client flows” on,
        delegated <code>Chat.Read</code>. Paste its Application (client) ID below.
      </p>

      <label className="jira-field">
        <span>Client ID</span>
        <input
          className="jira-input"
          value={clientId}
          onChange={(e) => setClientId(e.target.value)}
          placeholder="00000000-0000-0000-0000-000000000000"
          spellCheck={false}
          autoCapitalize="off"
        />
      </label>
      <label className="jira-field">
        <span>Tenant (optional)</span>
        <input
          className="jira-input"
          value={tenant}
          onChange={(e) => setTenant(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
          placeholder="organizations (default) — or yourcompany.com"
          spellCheck={false}
          autoCapitalize="off"
        />
      </label>

      <div className="jira-connect-actions">
        <button
          className="jira-token-link"
          onClick={() =>
            void openUrl(
              "https://learn.microsoft.com/entra/identity-platform/quickstart-register-app",
            )
          }
          title="How to register the app on portal.azure.com"
        >
          Register an app ↗
        </button>
        <button
          className="wb-cta wb-cta-sm"
          onClick={submit}
          disabled={connecting || !clientId.trim()}
        >
          {connecting ? "Starting…" : "Connect"}
        </button>
      </div>

      {connectError && <PanelError message={connectError} />}
    </div>
  );
}

function DeviceCodeScreen() {
  const login = useTeamsStore((s) => s.login)!;
  const cancelLogin = useTeamsStore((s) => s.cancelLogin);
  const showToast = useToastStore((s) => s.show);

  const copyCode = async () => {
    try {
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(login.userCode);
    } catch {
      await navigator.clipboard.writeText(login.userCode).catch(() => {});
    }
    showToast("Code copied", "info");
  };

  return (
    <div className="teams-devicecode">
      <p className="wb-hint">Sign in with your Microsoft account and enter this code when asked:</p>
      <button className="teams-usercode" onClick={() => void copyCode()} title="Click to copy">
        {login.userCode}
      </button>
      <div className="jira-connect-actions">
        <button
          className="wb-cta wb-cta-sm"
          onClick={() => void openUrl(login.verificationUri)}
          title={login.verificationUri}
        >
          Open sign-in page ↗
        </button>
        <button className="jira-token-link" onClick={() => void cancelLogin()}>
          Cancel
        </button>
      </div>
      <p className="wb-hint teams-waiting">Waiting for you to finish signing in…</p>
      <p className="wb-hint">
        If Microsoft shows “Need admin approval”, your organization blocks user consent — ask your
        IT admin to approve the app (it only reads your own chats).
      </p>
    </div>
  );
}

function ChatList() {
  const chats = useTeamsStore((s) => s.chats);
  const loadingChats = useTeamsStore((s) => s.loadingChats);
  const chatsError = useTeamsStore((s) => s.chatsError);
  const openChat = useTeamsStore((s) => s.openChat);

  if (chatsError) return <PanelError message={chatsError} />;
  if (loadingChats && chats.length === 0) return <div className="wb-hint">Loading chats…</div>;
  if (chats.length === 0) return <div className="wb-hint">No chats yet.</div>;

  return (
    <div className="teams-chatlist">
      {chats.map((c) => (
        <button className="teams-chat" key={c.id} onClick={() => void openChat(c.id)}>
          <span className="teams-chat-head">
            <span className="teams-chat-title">{c.title}</span>
            <span className="teams-chat-when">{formatWhen(c.lastActivity)}</span>
          </span>
          {c.lastPreview && <span className="teams-chat-preview">{c.lastPreview}</span>}
        </button>
      ))}
    </div>
  );
}

function MessagesView() {
  const activeChatId = useTeamsStore((s) => s.activeChatId);
  const chat = useTeamsStore((s) => s.chats.find((c) => c.id === activeChatId));
  const messages = useTeamsStore((s) => s.messages);
  const loadingMessages = useTeamsStore((s) => s.loadingMessages);
  const messagesError = useTeamsStore((s) => s.messagesError);
  const closeChat = useTeamsStore((s) => s.closeChat);

  return (
    <div className="teams-messages">
      <div className="teams-messages-head">
        <button className="jira-token-link" onClick={closeChat}>
          ← Chats
        </button>
        <span className="teams-chat-title">{chat?.title ?? ""}</span>
      </div>
      {messagesError ? (
        <PanelError message={messagesError} />
      ) : loadingMessages ? (
        <div className="wb-hint">Loading messages…</div>
      ) : messages.length === 0 ? (
        <div className="wb-hint">No messages here.</div>
      ) : (
        messages.map((m) => <MessageRow key={m.id} msg={m} />)
      )}
    </div>
  );
}

function MessageRow({ msg }: { msg: TeamsMessage }) {
  const showToast = useToastStore((s) => s.show);

  const copyBody = async () => {
    try {
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(msg.body);
    } catch {
      await navigator.clipboard.writeText(msg.body).catch(() => {});
    }
    showToast("Message copied", "info");
  };

  return (
    <div className="teams-msg">
      <div className="teams-msg-head">
        <span className="teams-msg-from">{msg.from}</span>
        <span className="teams-msg-when">{formatWhen(msg.created)}</span>
        <span className="teams-msg-actions">
          <button
            className="note-act"
            onClick={() => void typeIntoActiveSession(msg.body)}
            title="Type into the agent's input (doesn't send — you review first)"
          >
            ▸
          </button>
          <button className="note-act" onClick={() => void copyBody()} title="Copy message text">
            ⧉
          </button>
        </span>
      </div>
      <div className="teams-msg-body">{msg.body}</div>
    </div>
  );
}

/// Compact local timestamp: time for today, date+time otherwise. Empty input
/// (Graph omitted the field) renders nothing.
function formatWhen(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "";
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return sameDay
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
