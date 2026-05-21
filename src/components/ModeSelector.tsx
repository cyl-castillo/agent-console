import type { AgentMode } from "../types/domain";

interface Props {
  value: AgentMode;
  onChange: (m: AgentMode) => void;
  disabled?: boolean;
}

const MODES: { id: AgentMode; label: string; hint: string }[] = [
  { id: "plan",   label: "Plan",   hint: "Analyze only. No edits, no commands." },
  { id: "build",  label: "Build",  hint: "Implement changes carefully." },
  { id: "debug",  label: "Debug",  hint: "Diagnose the issue first; identify root cause." },
  { id: "review", label: "Review", hint: "Review current diff. No modifications." },
];

export function ModeSelector({ value, onChange, disabled }: Props) {
  return (
    <div className="mode-selector" role="radiogroup" aria-label="Agent mode">
      {MODES.map((m) => (
        <button
          key={m.id}
          type="button"
          role="radio"
          aria-checked={value === m.id}
          className={`mode-option ${value === m.id ? "active" : ""}`}
          onClick={() => onChange(m.id)}
          disabled={disabled}
          title={m.hint}
        >
          {m.label}
        </button>
      ))}
    </div>
  );
}
