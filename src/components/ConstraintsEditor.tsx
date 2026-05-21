import { useTaskStore } from "../stores/taskStore";

export function ConstraintsEditor() {
  const constraints = useTaskStore((s) => s.constraints);
  const draft = useTaskStore((s) => s.draftConstraint);
  const setDraft = useTaskStore((s) => s.setDraftConstraint);
  const add = useTaskStore((s) => s.addConstraint);
  const remove = useTaskStore((s) => s.removeConstraint);
  const clear = useTaskStore((s) => s.clearConstraints);

  return (
    <div className="constraints">
      <div className="constraints-header">
        <span className="constraints-label">Constraints</span>
        {constraints.length > 0 && (
          <button className="constraints-clear" onClick={clear} title="Clear all">clear</button>
        )}
      </div>
      <ul className="constraints-list">
        {constraints.map((c, i) => (
          <li key={i} className="constraint">
            <span>{c}</span>
            <button onClick={() => remove(i)} title="Remove" className="constraint-x">×</button>
          </li>
        ))}
      </ul>
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && draft.trim()) {
            e.preventDefault();
            add(draft);
          }
        }}
        placeholder="Add constraint, press Enter…"
        className="constraint-input"
      />
    </div>
  );
}
