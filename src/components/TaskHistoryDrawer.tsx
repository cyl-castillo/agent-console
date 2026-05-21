import { useEffect } from "react";

import { useTaskStore } from "../stores/taskStore";
import type { Task } from "../types/domain";

export function TaskHistoryDrawer() {
  const open = useTaskStore((s) => s.historyOpen);
  const close = useTaskStore((s) => s.toggleHistory);
  const items = useTaskStore((s) => s.history);
  const load = useTaskStore((s) => s.loadHistory);
  const del = useTaskStore((s) => s.deleteHistoryItem);

  useEffect(() => { if (open) load(); }, [open, load]);

  if (!open) return null;
  return (
    <div className="modal-backdrop" onClick={close}>
      <div className="modal history-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Task history</div>
        {items.length === 0
          ? <div className="placeholder" style={{ padding: 12 }}>No tasks recorded yet.</div>
          : (
            <ul className="history-list">
              {items.map((t) => <HistoryRow key={t.id} task={t} onDelete={() => del(t.id)} />)}
            </ul>
          )}
        <div className="modal-actions">
          <button onClick={close}>Close</button>
        </div>
      </div>
    </div>
  );
}

function HistoryRow({ task, onDelete }: { task: Task; onDelete: () => void }) {
  const when = new Date(task.createdAtMs).toLocaleString();
  const duration = task.completedAtMs
    ? `${((task.completedAtMs - task.createdAtMs) / 1000).toFixed(1)}s`
    : "—";
  const cost = task.costUsd != null ? `$${task.costUsd.toFixed(4)}` : "—";
  const statusCls = task.status === "completed" ? "ok" : task.status === "failed" ? "err" : "dim";

  return (
    <li className="history-row">
      <div className="history-head">
        <span className={`history-mode mode-${task.mode}`}>{task.mode}</span>
        <span className={`history-status history-status-${statusCls}`}>{task.status ?? "?"}</span>
        <span className="history-when">{when}</span>
        <span className="history-stat">{duration} · {cost}</span>
        <button className="history-delete" onClick={onDelete} title="Forget">×</button>
      </div>
      <div className="history-prompt">{task.prompt}</div>
      <div className="history-aggs">
        {task.filesRead.length > 0     && <span>▸ {task.filesRead.length} read</span>}
        {task.filesModified.length > 0 && <span>✎ {task.filesModified.length} modified</span>}
        {task.commandsExecuted.length > 0 && <span>$ {task.commandsExecuted.length} cmds</span>}
        {task.constraints.length > 0 && <span>· {task.constraints.length} constraint(s)</span>}
      </div>
    </li>
  );
}
