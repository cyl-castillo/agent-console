import { useEffect, useMemo, useState } from "react";

import { Modal } from "./Modal";
import { SetupDoctor } from "./SetupDoctor";
import { usePreflightStore, toolStatus } from "../stores/preflightStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useSessionStore } from "../stores/sessionStore";
import { startLoginSession } from "../lib/loginSession";
import { pickFolder } from "../ipc/tauri";
import type { AgentKind } from "../agents/profiles";

/// First-run wizard for the user who installed the app and nothing else.
/// Auto-opens only when NO agent CLI is present (see App). Every action goes
/// through the same review-first surfaces as the rest of the app — the wizard
/// just sequences them: pick an engine → install what's missing → log in →
/// start working. Progress is driven by live preflight data, so finishing a
/// step outside the wizard (e.g. installing in a terminal) still advances it.
type WizardStep = "choose" | "project" | "install" | "login" | "done";

export function WelcomeWizard({ onClose }: { onClose: () => void }) {
  const result = usePreflightStore((s) => s.result);
  const checking = usePreflightStore((s) => s.checking);
  const check = usePreflightStore((s) => s.check);
  const markWizardDone = useOnboardingStore((s) => s.markWizardDone);
  const project = useSessionStore((s) => s.project);
  const openProject = useSessionStore((s) => s.openProject);

  const [engine, setEngine] = useState<AgentKind | null>(null);

  useEffect(() => {
    void check();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const engineInstalled = engine ? !!toolStatus(result, engine)?.found : false;
  const engineAuth = result?.auth.find((a) => a.engine === engine);

  // The step is derived, never stored: install finished in a side terminal or
  // a login completed minutes later both advance the wizard on re-check.
  // A project must exist before install/login: those steps open terminal
  // sessions, and terminals only render inside a project layout — without one
  // the session would start invisibly.
  const step: WizardStep = useMemo(() => {
    if (!engine) return "choose";
    if (!project) return "project";
    if (!engineInstalled) return "install";
    if (engineAuth?.loggedIn !== true) return "login";
    return "done";
  }, [engine, project, engineInstalled, engineAuth]);

  const finish = () => {
    markWizardDone();
    onClose();
  };

  return (
    <Modal onClose={finish} className="welcome-wizard" ariaLabel="Welcome">
      <div className="gs-head">
        <div>
          <div className="gs-title">Welcome to Agent Console</div>
          <div className="gs-subtitle">
            {step === "choose" && "Let's get you a coding agent — four short steps."}
            {step === "project" && "Step 2 of 4 — open a project folder"}
            {step === "install" && "Step 3 of 4 — install the tools"}
            {step === "login" && "Step 4 of 4 — log in"}
            {step === "done" && "All set!"}
          </div>
        </div>
      </div>

      {step === "choose" && (
        <div className="wizard-body">
          <p className="gs-step-desc">
            Agent Console drives a coding agent in a real terminal. Which one do you want to start
            with? (You can add the other later.)
          </p>
          <div className="wizard-choices">
            <button className="wizard-choice" onClick={() => setEngine("claude")}>
              <div className="wizard-choice-title">Claude Code (recommended)</div>
              <div className="wizard-choice-desc">
                Anthropic's agent. Needs a Claude account (Pro/Max or API).
              </div>
            </button>
            <button className="wizard-choice" onClick={() => setEngine("codex")}>
              <div className="wizard-choice-title">Codex</div>
              <div className="wizard-choice-desc">
                OpenAI's agent. Needs a ChatGPT account. Requires Node.
              </div>
            </button>
          </div>
          <button className="wb-link wizard-skip" onClick={finish}>
            I'll set things up myself — skip
          </button>
        </div>
      )}

      {step === "project" && (
        <div className="wizard-body">
          <p className="gs-step-desc">
            Everything in Agent Console happens inside a project folder — the terminal, the agent,
            the history. Pick the folder you want to work in (an empty one is fine for trying things
            out).
          </p>
          <button
            className="wb-cta"
            onClick={() =>
              void pickFolder().then((path) => {
                if (path) return openProject(path);
              })
            }
          >
            Choose a folder…
          </button>
        </div>
      )}

      {step === "install" && (
        <div className="wizard-body">
          <p className="gs-step-desc">
            Hit <strong>Install</strong> on anything red. Each install opens a terminal with the
            official command already typed — read it, then press Enter. When everything you need is
            green, this step advances by itself on re-check.
          </p>
          <SetupDoctor />
        </div>
      )}

      {step === "login" && (
        <div className="wizard-body">
          <p className="gs-step-desc">
            <code>{engine}</code> is installed — now connect your account. A terminal will open with
            the login flow; it finishes in your browser.
          </p>
          <button
            className="wb-cta"
            onClick={() => startLoginSession(engine as AgentKind)}
            disabled={!engine}
          >
            Log in to {engine}
          </button>
          <button
            className="wb-link doctor-recheck"
            disabled={checking}
            onClick={() => void check()}
          >
            {checking ? "checking…" : "↻ I logged in — re-check"}
          </button>
        </div>
      )}

      {step === "done" && (
        <div className="wizard-body">
          <p className="gs-step-desc">
            <code>{engine}</code> is installed and logged in. Start a session and say hi — the agent
            launches automatically, and when your first prompt lands the console confirms its
            bridges are working.
          </p>
          <button className="wb-cta" onClick={finish}>
            Start working →
          </button>
        </div>
      )}

      {step !== "choose" && step !== "done" && (
        <div className="gs-foot">
          <button className="wb-link" onClick={() => setEngine(null)}>
            ← back
          </button>
          <button className="wb-link" onClick={finish}>
            skip for now
          </button>
        </div>
      )}
    </Modal>
  );
}
