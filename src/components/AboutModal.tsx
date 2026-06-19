import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Modal } from "./Modal";

const REPO_URL = "https://github.com/cyl-castillo/agent-console";
const SPONSORS_URL = "https://github.com/sponsors/cyl-castillo";
const BMC_URL = "https://www.buymeacoffee.com/cylcastillo";

interface Props {
  onClose: () => void;
}

export function AboutModal({ onClose }: Props) {
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  return (
    <Modal onClose={onClose} className="about-modal" ariaLabel="About Agent Console">
        <div className="about-head">
          <div className="about-title">Agent Console</div>
          <div className="about-version">{version ? `v${version}` : ""} · early preview</div>
        </div>

        <div className="about-quote">
          Built for the AI-native era of software engineering.
        </div>

        <dl className="about-fields">
          <dt>Stack</dt>
          <dd>Tauri 2 · Rust · React 19 · TypeScript</dd>
          <dt>Agent</dt>
          <dd>Claude Code CLI (stream-json)</dd>
          <dt>License</dt>
          <dd>MIT</dd>
        </dl>

        <div className="about-links">
          <a href={REPO_URL} target="_blank" rel="noopener">↗ GitHub</a>
          <a href={SPONSORS_URL} target="_blank" rel="noopener">♥ Sponsor</a>
          <a href={BMC_URL} target="_blank" rel="noopener">☕ Buy me a coffee</a>
        </div>

        <div className="modal-actions">
          <button onClick={onClose}>Close</button>
        </div>
    </Modal>
  );
}
