import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Modal } from "./Modal";
import { reportProblem } from "../lib/reportProblem";
import { ipc } from "../ipc/tauri";

const REPO_URL = "https://github.com/cyl-castillo/agent-console";
const SPONSORS_URL = "https://github.com/sponsors/cyl-castillo";
const BMC_URL = "https://www.buymeacoffee.com/cylcastillo";

interface Props {
  onClose: () => void;
}

export function AboutModal({ onClose }: Props) {
  const [version, setVersion] = useState("");
  const [build, setBuild] = useState("");
  const [diagNote, setDiagNote] = useState("");

  const copyDiagnostics = async () => {
    setDiagNote("collecting…");
    try {
      const text = await ipc.diagnosticsBundle();
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(text);
      setDiagNote("copied to clipboard");
    } catch (e) {
      setDiagNote(`failed: ${String(e).slice(0, 80)}`);
    }
  };

  const showLog = async () => {
    try {
      const file = await ipc.diagnosticsLogFile();
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(file);
    } catch (e) {
      setDiagNote(`no log: ${String(e).slice(0, 80)}`);
    }
  };

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(""));
    ipc
      .appBuildInfo()
      .then((b) => {
        const when = b.buildTimeSecs
          ? new Date(b.buildTimeSecs * 1000).toISOString().slice(0, 16).replace("T", " ") + " UTC"
          : "";
        setBuild(`${b.commit}${when ? ` · built ${when}` : ""}${b.debug ? " · debug" : ""}`);
      })
      .catch(() => setBuild(""));
  }, []);

  return (
    <Modal onClose={onClose} className="about-modal" ariaLabel="About Agent Console">
      <div className="about-head">
        <div className="about-title">Agent Console</div>
        <div className="about-version">
          {version ? `v${version}` : ""} · early preview
          {build && <span title="build provenance"> · {build}</span>}
        </div>
      </div>

      <div className="about-quote">Built for the AI-native era of software engineering.</div>

      <dl className="about-fields">
        <dt>Stack</dt>
        <dd>Tauri 2 · Rust · React 19 · TypeScript</dd>
        <dt>Agents</dt>
        <dd>Claude Code · Codex (per session)</dd>
        <dt>License</dt>
        <dd>AGPL-3.0-only</dd>
        <dt>Diagnostics</dt>
        <dd className="about-diagnostics">
          <button type="button" className="link-button" onClick={() => void copyDiagnostics()}>
            Copy diagnostics
          </button>
          {" · "}
          <button type="button" className="link-button" onClick={() => void showLog()}>
            Show log file
          </button>
          {diagNote && <span className="about-diag-note"> — {diagNote}</span>}
        </dd>
      </dl>

      <div className="about-links">
        <a href={REPO_URL} target="_blank" rel="noopener">
          ↗ GitHub
        </a>
        <a
          href="#report"
          onClick={(e) => {
            e.preventDefault();
            void reportProblem();
          }}
        >
          ⚑ Report a problem
        </a>
        <a href={SPONSORS_URL} target="_blank" rel="noopener">
          ♥ Sponsor
        </a>
        <a href={BMC_URL} target="_blank" rel="noopener">
          ☕ Buy me a coffee
        </a>
      </div>

      <div className="modal-actions">
        <button onClick={onClose}>Close</button>
      </div>
    </Modal>
  );
}
