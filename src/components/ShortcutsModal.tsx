import { Icon } from "./Icon";
import { Modal } from "./Modal";

const GROUPS = [
  {
    title: "Navigation",
    rows: [
      ["Ctrl+1", "Open Terminal"],
      ["Ctrl+2", "Open Changes"],
      ["Ctrl+3", "Open Preview"],
      ["Ctrl+B", "Toggle workspace sidebar"],
      ["Ctrl+J", "Toggle side panel"],
      ["Ctrl+P", "Open command palette"],
      ["Ctrl+/", "Show keyboard shortcuts"],
    ],
  },
  {
    title: "Sessions",
    rows: [
      ["Ctrl+T", "New session"],
      ["Ctrl+Tab", "Next live session"],
      ["Ctrl+Shift+Tab", "Previous live session"],
      ["Ctrl+]", "Next live session"],
      ["Ctrl+[", "Previous live session"],
    ],
  },
  {
    title: "Workflows",
    rows: [
      ["Ctrl+E", "Toggle the prompt composer (draft multi-line, Ctrl+Enter sends)"],
      ["Ctrl+L", "Clear terminal"],
      ["Ctrl+V", "Paste clipboard image into the agent"],
      ["Ctrl+Shift+V", "Toggle voice input on/off"],
      ["Ctrl+Space (hold)", "Push-to-talk: dictate into the composer"],
      ["Ctrl+R", "Refresh git changes"],
      ["Ctrl+W", "Close active session"],
      ["Esc", "Deny pending agent action or close modal"],
      ["Ctrl+Enter", "Approve pending agent action"],
      ["Ctrl+D", "Deny pending agent action"],
      ["Enter", "Send chat message"],
      ["Shift+Enter", "New line in chat input"],
    ],
  },
] as const;

export function ShortcutsModal({ onClose }: { onClose: () => void }) {
  return (
    <Modal onClose={onClose} className="shortcuts-modal" ariaLabel="Keyboard Shortcuts">
      <div className="shortcuts-head">
        <div>
          <div className="shortcuts-title">Keyboard Shortcuts</div>
          <div className="shortcuts-subtitle">Terminal-first controls for daily navigation.</div>
        </div>
        <button className="gs-close" onClick={onClose} title="Close" aria-label="Close">
          <Icon name="x" size={14} />
        </button>
      </div>

      <div className="shortcuts-groups">
        {GROUPS.map((group) => (
          <section className="shortcuts-group" key={group.title}>
            <div className="shortcuts-group-title">{group.title}</div>
            {group.rows.map(([keys, label]) => (
              <div className="shortcuts-row" key={keys}>
                <span className="shortcuts-label">{label}</span>
                <span className="shortcuts-keys">
                  {keys.split("+").map((k, i, all) => (
                    <span key={`${keys}-${k}-${i}`}>
                      <kbd>{k}</kbd>
                      {i < all.length - 1 && <span className="shortcut-plus">+</span>}
                    </span>
                  ))}
                </span>
              </div>
            ))}
          </section>
        ))}
      </div>
    </Modal>
  );
}
