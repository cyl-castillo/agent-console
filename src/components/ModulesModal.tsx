import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { MODULES } from "../lib/modules";
import { useModulesStore } from "../stores/modulesStore";

/// Switchboard for the workbench modules. Off = hidden (strip button, palette
/// actions, restored tab) — nothing is uninstalled and no data is touched, so
/// flipping a module back on restores it exactly as it was.
export function ModulesModal({ onClose }: { onClose: () => void }) {
  const disabled = useModulesStore((s) => s.disabled);
  const setEnabled = useModulesStore((s) => s.setEnabled);

  return (
    <Modal onClose={onClose} className="modules-modal" ariaLabel="Modules">
      <div className="shortcuts-head">
        <div>
          <div className="shortcuts-title">Modules</div>
          <div className="shortcuts-subtitle">
            Every part of the workbench is optional. Switch off what you don't use — it's only
            hidden, never deleted.
          </div>
        </div>
        <button className="gs-close" onClick={onClose} title="Close" aria-label="Close">
          <Icon name="x" size={14} />
        </button>
      </div>

      <div className="modules-list">
        {MODULES.map((m) => {
          const on = !disabled.includes(m.key);
          return (
            <div className="module-row" key={m.key}>
              <div className="module-info">
                <div className="module-label">{m.label}</div>
                <div className="module-desc">{m.description}</div>
              </div>
              {m.locked ? (
                <span className="module-locked" title="Safety modules can't be switched off">
                  <Icon name="lock" size={12} /> Always on
                </span>
              ) : (
                <button
                  role="switch"
                  aria-checked={on}
                  aria-label={`${m.label} module`}
                  className={`module-switch ${on ? "on" : ""}`}
                  onClick={() => setEnabled(m.key, !on)}
                >
                  <span className="module-switch-knob" />
                </button>
              )}
            </div>
          );
        })}
      </div>
    </Modal>
  );
}
