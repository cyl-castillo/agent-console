import { useEffect } from "react";
import { KeyRound, Loader2, ShieldQuestion, TerminalSquare, Wrench } from "lucide-react";
import { startLoginSession } from "../../../lib/loginSession";
import { useApprovalStore } from "../../../stores/approvalStore";
import { capFor, useBudgetStore } from "../../../stores/budgetStore";
import { useLiveStatusStore } from "../../../stores/liveStatusStore";
import { useOverlaysStore } from "../../../stores/overlaysStore";
import { useShellStore } from "../../../stores/shellStore";
import { isBooting, signalsFor, useTermSignalsStore } from "../../../stores/termSignalsStore";
import type { TerminalSession } from "../../../stores/terminalsStore";
import { useToastStore } from "../../../stores/toastStore";
import { Button } from "../../../ui/Button";
import { ApprovalDecision } from "../ApprovalDecision";
import { BudgetDecision } from "../BudgetDecision";

/// F3: what only the terminal used to show, as cards in the thread — the
/// CLI booting, a trust question, a login ask, a missing command, the
/// approvals and budget stops of THIS session. The Inbox stays the global
/// tray; here they sit in the conversation they belong to. Cards help and
/// point; they never answer for the user.
export function SessionCards({
  session,
  now,
  lastTurnTs,
}: {
  session: TerminalSession;
  now: number;
  /// When the newest witnessed prompt of this session was recorded.
  lastTurnTs: number | null;
}) {
  // The Sessions zone stays mounted while another destination is up; the
  // cards (and the approval cards they share with the Inbox) must not exist
  // twice in the DOM, so they render only while Sessions is the stage.
  const onStage = useShellStore((s) => s.destination === "sessions");
  const sig = useTermSignalsStore((s) => signalsFor(s.bySession, session.id));
  const clear = useTermSignalsStore((s) => s.clear);
  const drawerOpen = useShellStore((s) => s.drawerOpen);
  const toggleDrawer = useShellStore((s) => s.toggleDrawer);
  const showOverlay = useOverlaysStore((s) => s.show);
  const showToast = useToastStore((s) => s.show);
  const approvals = useApprovalStore((s) => s.queue);
  const mine = approvals.filter((r) => r.termId === session.id);
  const budgets = useBudgetStore();
  const spent = useLiveStatusStore((s) => s.byTerm[session.id]?.costUsd);
  const breached = budgets.breached[session.id];

  // A prompt witnessed AFTER a question means the CLI is up and the question
  // is behind us (older turns say nothing about a question asked just now).
  useEffect(() => {
    if (lastTurnTs === null) return;
    const asked = Math.max(sig.bootedAt ?? 0, sig.trust ?? 0, sig.login ?? 0);
    if (asked > 0 && lastTurnTs > asked) clear(session.id);
  }, [lastTurnTs, sig.bootedAt, sig.trust, sig.login, clear, session.id]);

  const openTerminal = () => {
    if (!drawerOpen) toggleDrawer();
    window.dispatchEvent(new CustomEvent("ac:focus-terminal"));
  };
  const booting = isBooting(sig, now) && !sig.trust && !sig.login && !sig.notFound;
  const live = session.status === "live";
  if (!onStage) return null;

  return (
    <>
      {live && booting && (
        <Card
          testid="card-boot"
          icon={<Loader2 size={16} className="animate-spin" aria-hidden />}
          title={`Launching ${session.agent ?? "claude"}…`}
          body="Waiting for the CLI prompt. Anything you send now is queued and goes out when it is up."
          actions={
            <Button
              size="sm"
              variant="ghost"
              icon={<TerminalSquare size={14} />}
              onClick={openTerminal}
            >
              Open terminal
            </Button>
          }
        />
      )}
      {live && sig.trust && (
        <Card
          testid="card-trust"
          tone="warn"
          icon={<ShieldQuestion size={16} aria-hidden />}
          title="Claude asks whether you trust this folder"
          body="The question is in the terminal and only you can answer it — nothing runs until you do."
          actions={
            <Button
              size="sm"
              variant="primary"
              icon={<TerminalSquare size={14} />}
              onClick={openTerminal}
            >
              Answer in the terminal
            </Button>
          }
        />
      )}
      {live && sig.login && (
        <Card
          testid="card-login"
          tone="warn"
          icon={<KeyRound size={16} aria-hidden />}
          title="The CLI needs you to log in"
          body="It will open a browser for the OAuth flow; this session resumes once you are in."
          actions={
            <>
              <Button
                size="sm"
                variant="primary"
                onClick={() => {
                  void startLoginSession(session.agent ?? "claude");
                  showToast("Login session started — follow the browser", "info");
                }}
              >
                Log in
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon={<TerminalSquare size={14} />}
                onClick={openTerminal}
              >
                Open terminal
              </Button>
            </>
          }
        />
      )}
      {sig.notFound && (
        <Card
          testid="card-notfound"
          tone="danger"
          icon={<Wrench size={16} aria-hidden />}
          title={`${session.agent ?? "claude"} is not installed, or not on this shell's PATH`}
          body="The setup wizard installs the CLI and checks the prerequisites; the terminal shows the exact error."
          actions={
            <>
              <Button size="sm" variant="primary" onClick={() => showOverlay("wizard")}>
                Open setup
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon={<TerminalSquare size={14} />}
                onClick={openTerminal}
              >
                Open terminal
              </Button>
            </>
          }
        />
      )}
      {mine.length > 0 && (
        <div data-testid="inline-approvals" className="border-t border-border">
          {mine.map((r, i) => (
            <ApprovalDecision key={r.id} req={r} index={i} total={mine.length} />
          ))}
        </div>
      )}
      {breached !== undefined && (
        <div className="border-t border-border">
          <BudgetDecision
            session={session}
            spent={spent ?? breached}
            cap={capFor(budgets, session.id)}
          />
        </div>
      )}
    </>
  );
}

function Card({
  testid,
  tone = "neutral",
  icon,
  title,
  body,
  actions,
}: {
  testid: string;
  tone?: "neutral" | "warn" | "danger";
  icon: React.ReactNode;
  title: string;
  body: string;
  actions?: React.ReactNode;
}) {
  const border =
    tone === "warn" ? "border-warn" : tone === "danger" ? "border-danger" : "border-border";
  return (
    <div
      className={`mx-4 my-2 flex flex-col gap-2 rounded-3 border ${border} bg-surface-1 px-3 py-2 font-sans text-body`}
      data-testid={testid}
      role="status"
    >
      <div className="flex items-center gap-2 font-medium text-foreground">
        <span className="text-muted-foreground">{icon}</span>
        {title}
      </div>
      <p className="m-0 text-small text-muted-foreground">{body}</p>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}
