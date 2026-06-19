import { useEffect, useMemo } from "react";

import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { useSessionStore } from "../stores/sessionStore";
import { useOnboardingStore } from "../stores/onboardingStore";
import { useAdvisorStore } from "../stores/advisorStore";
import { useSkillsStore } from "../stores/skillsStore";

interface Props {
  onClose: () => void;
  /// Called when a checklist item asks the user to switch to a workbench tab.
  onJumpToTab: (tab: "skills" | "permissions" | "advisor") => void;
}

interface Step {
  key: string;
  title: string;
  description: string;
  done: boolean;
  action?: { label: string; onClick: () => void };
}

export function GettingStartedModal({ onClose, onJumpToTab }: Props) {
  const project = useSessionStore((s) => s.project);
  const onboarding = useOnboardingStore();
  const advisorItems = useAdvisorStore((s) => s.items);
  const installedSkills = useSkillsStore((s) => s.installed);
  const recentPromptsCount = useSkillsStore((s) => s.recent.length);

  // Mark seen once when opened so it doesn't auto-open again on next launch.
  useEffect(() => {
    if (!onboarding.seenWelcome) onboarding.markSeenWelcome();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const steps: Step[] = useMemo(() => [
    {
      key: "open-project",
      title: "Abrí un proyecto",
      description: "Cada terminal arranca con el cwd del proyecto activo.",
      done: !!project,
    },
    {
      key: "talk-to-claude",
      title: "Charlá con Claude en la terminal",
      description: "Cada terminal nueva auto-ejecuta `claude`. Escribí una pregunta y dale enter.",
      done: onboarding.promptedClaude || recentPromptsCount > 0,
    },
    {
      key: "advisor",
      title: "Generá skills con el Advisor",
      description: "El Advisor analiza tu proyecto y sugiere skills concretas. Click para abrir.",
      done: onboarding.triggeredAdvisor || advisorItems.length > 0,
      action: { label: "Abrir Advisor →", onClick: () => { onJumpToTab("advisor"); onClose(); } },
    },
    {
      key: "create-skill",
      title: "Creá tu primera skill",
      description: "Desde la lista del Advisor, revisá una recomendación y dale Create.",
      done: onboarding.createdSkill || installedSkills.length > 0,
      action: { label: "Ver skills →", onClick: () => { onJumpToTab("skills"); onClose(); } },
    },
    {
      key: "permissions",
      title: "Revisá tus permisos",
      description: "Mirá qué herramientas Claude puede usar sin pedirte y ajustá según necesites.",
      done: onboarding.visitedPermissions,
      action: { label: "Abrir Permissions →", onClick: () => { onJumpToTab("permissions"); onClose(); } },
    },
  ], [project, onboarding, advisorItems.length, installedSkills.length, recentPromptsCount, onJumpToTab, onClose]);

  const completed = steps.filter((s) => s.done).length;

  return (
    <Modal onClose={onClose} className="getting-started-modal" ariaLabel="Getting Started">
        <div className="gs-head">
          <div>
            <div className="gs-title">Getting Started</div>
            <div className="gs-subtitle">
              {completed} de {steps.length} completados
            </div>
          </div>
          <button className="gs-close" onClick={onClose} title="Cerrar" aria-label="Cerrar">
            <Icon name="x" size={14} />
          </button>
        </div>

        <div className="gs-progress">
          <div className="gs-progress-fill" style={{ width: `${(completed / steps.length) * 100}%` }} />
        </div>

        <ul className="gs-steps">
          {steps.map((step) => (
            <li key={step.key} className={`gs-step ${step.done ? "done" : ""}`}>
              <span className="gs-check" aria-hidden>{step.done ? "✓" : "○"}</span>
              <div className="gs-step-body">
                <div className="gs-step-title">{step.title}</div>
                <div className="gs-step-desc">{step.description}</div>
              </div>
              {step.action && !step.done && (
                <button className="wb-cta wb-cta-sm" onClick={step.action.onClick}>
                  {step.action.label}
                </button>
              )}
            </li>
          ))}
        </ul>

        <div className="gs-foot">
          <p className="gs-hint">
            Esta guía vive en el icono <strong>?</strong> del topbar. Volvé cuando quieras.
          </p>
          <button onClick={onClose}>Cerrar</button>
        </div>
    </Modal>
  );
}
