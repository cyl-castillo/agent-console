import { useState } from "react";

import { ipc, pickOpenFile, pickSaveFile } from "../ipc/tauri";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { useSchedulerStore } from "../stores/schedulerStore";
import { useRoundtableStore } from "../stores/roundtableStore";
import { useSkillsStore } from "../stores/skillsStore";
import { useContextStore } from "../stores/contextStore";
import type {
  ExportOptions,
  ImportDecision,
  ImportDecisions,
  ImportManifest,
} from "../types/domain";

// The four exportable blocks (learning bundles skills + memory into one file
// block; it splits back into two decisions on import). Each has a one-line gloss
// note so the user knows what does NOT travel.
const EXPORT_BLOCKS: { key: keyof ExportOptions; label: string; desc: string }[] = [
  { key: "sessions", label: "Sessions", desc: "Terminals + scrollback (resume ids are dropped)" },
  { key: "rooms", label: "Rooms", desc: "Roundtable transcripts (live run state is dropped)" },
  {
    key: "schedules",
    label: "Schedules",
    desc: "Scheduled jobs — imported disabled, never auto-fire",
  },
  { key: "learning", label: "Learning", desc: "Project skills + memory entries" },
];

// The five blocks the import side reasons about (learning is already split).
const IMPORT_BLOCKS: { key: keyof ImportDecisions; label: string }[] = [
  { key: "sessions", label: "Sessions" },
  { key: "rooms", label: "Rooms" },
  { key: "schedules", label: "Schedules" },
  { key: "skills", label: "Skills" },
  { key: "memory", label: "Memory" },
];

const DECISIONS: ImportDecision[] = ["skip", "merge", "replace"];

function basename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? "project";
}

export function ExportImportPanel() {
  const projectRoot = useSessionStore((s) => s.project?.root ?? null);
  const toast = useToastStore((s) => s.show);

  if (!projectRoot) {
    return (
      <div className="workbench">
        <div className="workbench-header workbench-header-slim">
          <span className="workbench-title">export / import</span>
        </div>
        <div className="workbench-body">
          <section className="wb-section">
            <p className="wb-hint">Open a project to export or import its work.</p>
          </section>
        </div>
      </div>
    );
  }

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">export / import</span>
      </div>
      <div className="workbench-body">
        <ExportSection projectRoot={projectRoot} toast={toast} />
        <ImportSection projectRoot={projectRoot} toast={toast} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

function ExportSection({
  projectRoot,
  toast,
}: {
  projectRoot: string;
  toast: (m: string, t?: "info" | "success" | "error") => void;
}) {
  const [opts, setOpts] = useState<ExportOptions>({
    sessions: true,
    rooms: true,
    schedules: true,
    learning: true,
    includeActivity: false,
  });
  const [busy, setBusy] = useState(false);

  const anyChosen = opts.sessions || opts.rooms || opts.schedules || opts.learning;

  async function doExport() {
    const dest = await pickSaveFile(`${basename(projectRoot)}-work.acwork`);
    if (!dest) return;
    setBusy(true);
    try {
      const r = await ipc.exportWork(projectRoot, opts, dest);
      toast(
        `Exported ${r.sessions} sessions, ${r.rooms} rooms, ${r.schedules} jobs, ${r.skills} skills, ${r.memory} memories`,
        "success",
      );
    } catch (e) {
      toast(`Export failed: ${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="wb-section">
      <div className="wb-section-title">export your work</div>
      <p className="wb-hint">
        Bundle the chosen blocks into a portable <code>.acwork</code> file. Secrets and
        machine-specific paths never leave — only the work itself.
      </p>
      <div className="wb-ei-blocks">
        {EXPORT_BLOCKS.map((b) => (
          <label className="wb-ei-check" key={b.key}>
            <input
              type="checkbox"
              checked={opts[b.key] as boolean}
              onChange={(e) => setOpts({ ...opts, [b.key]: e.target.checked })}
            />
            <span className="wb-ei-check-body">
              <span className="wb-ei-check-label">{b.label}</span>
              <span className="wb-ei-check-desc">{b.desc}</span>
            </span>
          </label>
        ))}
      </div>
      {opts.learning && (
        <label className="wb-ei-check wb-ei-advanced">
          <input
            type="checkbox"
            checked={opts.includeActivity}
            onChange={(e) => setOpts({ ...opts, includeActivity: e.target.checked })}
          />
          <span className="wb-ei-check-body">
            <span className="wb-ei-check-label">Include activity ledger</span>
            <span className="wb-ei-check-desc">
              Advanced — a detailed record of your prompts. Off by default.
            </span>
          </span>
        </label>
      )}
      <button className="wb-cta" onClick={doExport} disabled={busy || !anyChosen}>
        {busy ? "Exporting…" : "Export to file…"}
      </button>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

function ImportSection({
  projectRoot,
  toast,
}: {
  projectRoot: string;
  toast: (m: string, t?: "info" | "success" | "error") => void;
}) {
  const [srcPath, setSrcPath] = useState<string | null>(null);
  const [manifest, setManifest] = useState<ImportManifest | null>(null);
  const [decisions, setDecisions] = useState<ImportDecisions>({
    sessions: "skip",
    rooms: "skip",
    schedules: "skip",
    skills: "skip",
    memory: "skip",
  });
  const [busy, setBusy] = useState(false);

  async function chooseFile() {
    const src = await pickOpenFile();
    if (!src) return;
    setBusy(true);
    try {
      const m = await ipc.importWorkPreview(projectRoot, src);
      setSrcPath(src);
      setManifest(m);
      // Default a present block to "merge" (additive, safe) and an absent one to
      // "skip" — the user can change any of them before applying.
      setDecisions({
        sessions: m.sessions.present ? "merge" : "skip",
        rooms: m.rooms.present ? "merge" : "skip",
        schedules: m.schedules.present ? "merge" : "skip",
        skills: m.skills.present ? "merge" : "skip",
        memory: m.memory.present ? "merge" : "skip",
      });
    } catch (e) {
      toast(`Could not read that file: ${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  function reset() {
    setSrcPath(null);
    setManifest(null);
  }

  async function doImport() {
    if (!srcPath) return;
    setBusy(true);
    try {
      const r = await ipc.importWorkApply(projectRoot, srcPath, decisions);
      // Refresh the stores whose data we just changed. Terminals are NOT
      // re-hydrated live (that would disrupt running sessions) — imported
      // sessions appear next time the project is reopened.
      await Promise.allSettled([
        useSchedulerStore.getState().refresh(),
        useRoundtableStore.getState().loadRooms(),
        useSkillsStore.getState().refresh(),
        useContextStore.getState().refresh(),
      ]);
      const sessionsNote = r.sessions > 0 ? " (sessions appear on next project reopen)" : "";
      toast(
        `Imported ${r.sessions} sessions, ${r.rooms} rooms, ${r.schedules} jobs, ${r.skills} skills, ${r.memory} memories${sessionsNote}`,
        "success",
      );
      reset();
    } catch (e) {
      toast(`Import failed: ${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  if (!manifest) {
    return (
      <section className="wb-section">
        <div className="wb-section-title">import from a file</div>
        <p className="wb-hint">
          Load an <code>.acwork</code> file someone shared. You choose what to do with each block
          before anything is written.
        </p>
        <button className="wb-cta" onClick={chooseFile} disabled={busy}>
          {busy ? "Reading…" : "Choose a file…"}
        </button>
      </section>
    );
  }

  const anyApply = IMPORT_BLOCKS.some((b) => decisions[b.key] !== "skip");

  return (
    <section className="wb-section">
      <div className="wb-section-title">
        import from <span className="wb-ei-src">{manifest.sourceProjectName}</span>
      </div>
      <p className="wb-hint">Choose what to do with each block, then apply.</p>
      <div className="wb-ei-blocks">
        {IMPORT_BLOCKS.map((b) => {
          const m = manifest[b.key];
          if (!m.present) {
            return (
              <div className="wb-ei-row disabled" key={b.key}>
                <span className="wb-ei-row-label">{b.label}</span>
                <span className="wb-ei-row-meta">not in file</span>
              </div>
            );
          }
          return (
            <div className="wb-ei-row" key={b.key}>
              <span className="wb-ei-row-label">{b.label}</span>
              <span className="wb-ei-row-meta">
                {m.total} item{m.total === 1 ? "" : "s"}
                {m.collisions > 0 && (
                  <span className="wb-ei-collide" title="Already present in this project">
                    {" "}
                    · {m.collisions} overlap
                  </span>
                )}
              </span>
              <SegChooser
                value={decisions[b.key]}
                onChange={(v) => setDecisions({ ...decisions, [b.key]: v })}
              />
            </div>
          );
        })}
      </div>
      <div className="wb-ei-actions">
        <button className="wb-cta" onClick={doImport} disabled={busy || !anyApply}>
          {busy ? "Importing…" : "Apply import"}
        </button>
        <button className="workbench-action" onClick={reset} disabled={busy}>
          Cancel
        </button>
      </div>
    </section>
  );
}

function SegChooser({
  value,
  onChange,
}: {
  value: ImportDecision;
  onChange: (v: ImportDecision) => void;
}) {
  return (
    <div className="wb-ei-seg" role="group">
      {DECISIONS.map((d) => (
        <button
          key={d}
          className={`wb-ei-seg-btn ${value === d ? "active" : ""}`}
          onClick={() => onChange(d)}
          title={
            d === "skip"
              ? "Don't import this block"
              : d === "merge"
                ? "Add new items, keep anything already here"
                : "Overwrite overlapping items"
          }
        >
          {d}
        </button>
      ))}
    </div>
  );
}
