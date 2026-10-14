import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "../ipc/tauri";
import { DEFAULT_AGENT, type AgentKind } from "../agents/profiles";
import { createSession } from "./createSession";
import { sessionAttention } from "./attention";
import { bridgeActions, bridgeState } from "./agentBridge";
import { allPaletteActionIds, runPaletteAction } from "../stores/paletteStore";
import { useAgentStatusStore } from "../stores/agentStatusStore";
import { useApprovalStore } from "../stores/approvalStore";
import { useModelStore } from "../stores/modelStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";

/// A tool call the Rust side could not answer itself (`console://call`):
/// the stores are the truth for sessions, actions and where the shell is.
export interface ConsoleCall {
  id: number;
  caller: string;
  tool: string;
  arguments: Record<string, unknown>;
}

export type ConsoleAnswer = { ok: true; result: unknown } | { ok: false; error: string };

/// Mirror of console_mcp::action_allowed — enforced on both sides.
export function actionAllowed(id: string): boolean {
  return (
    id.startsWith("nav.") ||
    id.startsWith("ui.") ||
    [
      "help.getting_started",
      "git.refresh",
      "app.check_updates",
      "prompt.composer",
      "modules.configure",
      "session.new",
    ].includes(id)
  );
}

const str = (args: Record<string, unknown>, key: string): string | undefined =>
  typeof args[key] === "string" && (args[key] as string).length > 0
    ? (args[key] as string)
    : undefined;

/// Answer one call. Pure over the stores (no DOM except send_prompt's event),
/// so it is testable in node.
export async function handleConsoleCall(call: ConsoleCall): Promise<ConsoleAnswer> {
  const args = call.arguments ?? {};
  try {
    switch (call.tool) {
      case "console_state":
        return { ok: true, result: bridgeState() };
      case "list_actions":
        return { ok: true, result: bridgeActions() };
      case "invoke_action": {
        const id = str(args, "id");
        if (!id) return { ok: false, error: "Missing action id" };
        if (!allPaletteActionIds().includes(id))
          return { ok: false, error: `Unknown action ${id}` };
        if (!actionAllowed(id)) return { ok: false, error: `Action ${id} stays a human gesture` };
        const ran = await runPaletteAction(id);
        return ran
          ? { ok: true, result: { id, ran: true } }
          : { ok: false, error: `Action ${id} is not available right now` };
      }
      case "list_sessions": {
        const t = useTerminalsStore.getState();
        const queue = useApprovalStore.getState().queue;
        const status = useAgentStatusStore.getState();
        const now = Date.now();
        const sessions = t.sessions
          .filter((s) => !s.archived)
          .map((s) => ({
            id: s.id,
            name: s.name,
            status: s.status,
            active: s.id === t.activeId,
            agent: s.agent ?? null,
            model: s.model ?? null,
            worktree: s.worktree?.branch ?? null,
            lastActiveMs: s.lastActiveMs ?? s.createdAtMs,
            attention: sessionAttention({
              session: s,
              approvalTermIds: queue.map((r) => r.termId).filter((x): x is string => Boolean(x)),
              waitingTermId: status.waiting?.termId ?? null,
              workingUntil: status.workingUntil,
              workingTermId: null,
              activeId: t.activeId,
              now,
            }),
          }));
        return { ok: true, result: { sessions } };
      }
      case "new_session": {
        const project = useSessionStore.getState().project;
        if (!project) return { ok: false, error: "No project is open" };
        const agent = (str(args, "agent") ??
          useModelStore.getState().defaultAgentFor(project.root) ??
          DEFAULT_AGENT) as AgentKind;
        if (agent !== "claude" && agent !== "codex")
          return { ok: false, error: "agent must be claude or codex" };
        const before = new Set(useTerminalsStore.getState().sessions.map((s) => s.id));
        const ok = await createSession(agent, str(args, "model"));
        if (!ok) return { ok: false, error: "The session could not be started" };
        const created = useTerminalsStore.getState().sessions.find((s) => !before.has(s.id));
        if (!created) return { ok: false, error: "The session did not appear" };
        const name = str(args, "name");
        if (name) useTerminalsStore.getState().rename(created.id, name);
        return { ok: true, result: { id: created.id, name: name ?? created.name, agent } };
      }
      case "send_prompt": {
        const id = str(args, "session");
        const text = str(args, "text");
        if (!id || !text) return { ok: false, error: "session and text are required" };
        const session = useTerminalsStore.getState().sessions.find((s) => s.id === id);
        if (!session) return { ok: false, error: `Unknown session ${id}` };
        if (session.status !== "live") return { ok: false, error: `Session ${id} is not live` };
        const submit = args.submit !== false;
        if (typeof window === "undefined") return { ok: false, error: "No terminal host" };
        window.dispatchEvent(
          new CustomEvent("ac:term-input", {
            detail: { sessionId: id, data: submit ? `${text}\r` : text },
          }),
        );
        return { ok: true, result: { session: id, submitted: submit } };
      }
      default:
        return { ok: false, error: `Unknown tool ${call.tool}` };
    }
  } catch (e) {
    return { ok: false, error: String(e).slice(0, 300) };
  }
}

/// Mount once: every `console://call` is answered through `console_reply`.
export async function attachConsoleMcpHost(): Promise<UnlistenFn> {
  return listen<ConsoleCall>("console://call", (e) => {
    const call = e.payload;
    void handleConsoleCall(call).then((answer) =>
      ipc.consoleReply(call.id, answer).catch(() => {}),
    );
  });
}
