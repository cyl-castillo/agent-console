import { useEffect, useMemo, useRef } from "react";

import type { ChatBlock, Task } from "../types/domain";
import { useChatStore } from "../stores/chatStore";
import { useTaskStore } from "../stores/taskStore";
import { MarkdownText } from "./MarkdownText";
import { ModeSelector } from "./ModeSelector";
import { ConstraintsEditor } from "./ConstraintsEditor";

export function AgentChat() {
  const { blocks, sending, inputDraft, totalCost, approveAll, setDraft, send, reset, restoreSnapshot } =
    useChatStore();
  const tasks = useTaskStore((s) => s.tasks);
  const mode = useTaskStore((s) => s.mode);
  const setMode = useTaskStore((s) => s.setMode);
  const toggleHistory = useTaskStore((s) => s.toggleHistory);

  const listRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  // Group blocks by taskId for rendering.
  const groups = useMemo(() => groupByTask(tasks, blocks), [tasks, blocks]);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [blocks, tasks]);

  useEffect(() => {
    const handler = () => inputRef.current?.focus();
    window.addEventListener("ac:focus-chat", handler);
    return () => window.removeEventListener("ac:focus-chat", handler);
  }, []);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    send(inputDraft);
  };

  return (
    <div className="agent">
      <div className="agent-bar">
        <span className="agent-meta">claude · cli</span>
        {totalCost > 0 && <span className="agent-meta">· total ${totalCost.toFixed(4)}</span>}
        {approveAll && <span className="agent-meta" style={{ color: "var(--danger)" }}>· auto-approve</span>}
        <span className="spacer" />
        <button onClick={toggleHistory} title="Task history">History</button>
        <button onClick={reset} disabled={tasks.length === 0 && !sending}>Reset</button>
      </div>

      <div className="agent-messages" ref={listRef}>
        {tasks.length === 0 && (
          <div className="placeholder">
            Direct the agent. Pick a mode below, optionally add constraints, then type a task.
            Edits land in the Changes tab; a snapshot is taken before each turn so you can restore it.
          </div>
        )}
        {groups.map(({ task, items }) => (
          <TaskCard key={task.id} task={task} blocks={items} onRestore={restoreSnapshot} />
        ))}
      </div>

      <div className="task-context">
        <ModeSelector value={mode} onChange={setMode} disabled={sending} />
        <ConstraintsEditor />
      </div>

      <form className="agent-input" onSubmit={onSubmit}>
        <textarea
          ref={inputRef}
          value={inputDraft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              onSubmit(e as unknown as React.FormEvent);
            }
          }}
          placeholder="Direct the agent…  (Enter to send, Shift+Enter for newline)"
          disabled={sending}
          rows={3}
        />
        <div className="agent-actions">
          {sending
            ? <button type="button" onClick={reset}>Stop</button>
            : <button type="submit" className="primary" disabled={!inputDraft.trim()}>Send</button>
          }
        </div>
      </form>
    </div>
  );
}

function groupByTask(tasks: Task[], blocks: ChatBlock[]): Array<{ task: Task; items: ChatBlock[] }> {
  const map = new Map<string, ChatBlock[]>();
  for (const t of tasks) map.set(t.id, []);
  for (const b of blocks) {
    const list = map.get(b.taskId);
    if (list) list.push(b);
  }
  return tasks.map((task) => ({ task, items: map.get(task.id) ?? [] }));
}

function TaskCard({ task, blocks, onRestore }: {
  task: Task;
  blocks: ChatBlock[];
  onRestore: (sha: string, userBlockId: string) => void;
}) {
  const userBlock = blocks.find((b): b is Extract<ChatBlock, { kind: "user" }> => b.kind === "user");
  const others = blocks.filter((b) => b.kind !== "user");
  const isRunning = task.status === "running" || task.status === undefined;
  const duration = task.completedAtMs ? (task.completedAtMs - task.createdAtMs) / 1000 : null;

  return (
    <div className={`task-card task-${task.status ?? "running"}`}>
      <div className="task-head">
        <span className={`task-mode mode-${task.mode}`}>{task.mode}</span>
        <span className="task-prompt">{task.prompt}</span>
        {isRunning && <span className="caret" />}
        {userBlock?.snapshot && !userBlock.restored && (
          <button
            className="msg-restore"
            title="Restore working tree to before this turn"
            onClick={() => {
              if (userBlock.snapshot && confirm("Restore to before this turn? Uncommitted changes will be lost.")) {
                onRestore(userBlock.snapshot.commitSha, userBlock.id);
              }
            }}
          >↶ restore</button>
        )}
        {userBlock?.restored && <span className="msg-restored">· restored</span>}
      </div>

      {task.constraints.length > 0 && (
        <div className="task-constraints">
          {task.constraints.map((c, i) => <span key={i} className="task-constraint">{c}</span>)}
        </div>
      )}

      <div className="task-events">
        {others.map((b) => <BlockView key={b.id} block={b} />)}
      </div>

      {!isRunning && (
        <div className="task-summary">
          <span className={`task-summary-status status-${task.status}`}>
            {task.status === "completed" ? "✓ Done" : task.status === "failed" ? "✗ Failed" : "● Cancelled"}
          </span>
          {duration !== null && <span>· {duration.toFixed(1)}s</span>}
          {task.costUsd != null && <span>· ${task.costUsd.toFixed(4)}</span>}
          <div className="task-summary-aggs">
            {task.filesRead.length > 0 &&
              <span title={task.filesRead.join("\n")}>▸ Read {task.filesRead.length}</span>}
            {task.filesModified.length > 0 &&
              <span title={task.filesModified.join("\n")}>✎ Modified {task.filesModified.length}</span>}
            {task.commandsExecuted.length > 0 &&
              <span title={task.commandsExecuted.join("\n")}>$ Ran {task.commandsExecuted.length}</span>}
          </div>
        </div>
      )}
    </div>
  );
}

function BlockView({ block }: { block: ChatBlock }) {
  switch (block.kind) {
    case "text":
      return (
        <div className="msg msg-assistant">
          <MarkdownText content={block.content} />
        </div>
      );
    case "thinking":
      return (
        <div className="msg msg-thinking">
          <div className="msg-role">thinking</div>
          <div className="msg-body">{block.content}</div>
        </div>
      );
    case "tool":
      return <ToolBlock block={block} />;
    case "user":
      // User block is rendered in the card header.
      return null;
  }
}

function ToolBlock({ block }: { block: Extract<ChatBlock, { kind: "tool" }> }) {
  const icon =
    block.status === "running" ? "▸" :
    block.status === "ok"      ? "✓" : "✗";
  return (
    <div className={`tool tool-${block.status}`}>
      <span className="tool-icon">{icon}</span>
      <span className="tool-name">{block.name}</span>
      <span className="tool-input">{formatInput(block.name, block.input)}</span>
      {block.summary && <div className="tool-summary">{block.summary}</div>}
    </div>
  );
}

function formatInput(name: string, input: unknown): string {
  if (!input || typeof input !== "object") return "";
  const obj = input as Record<string, unknown>;
  if (name === "Bash" && typeof obj.command === "string") return obj.command;
  if (typeof obj.file_path === "string") return obj.file_path;
  if (typeof obj.path === "string") return obj.path;
  if (typeof obj.pattern === "string") return obj.pattern;
  if (typeof obj.url === "string") return obj.url;
  const j = JSON.stringify(obj);
  return j.length > 80 ? j.slice(0, 80) + "…" : j;
}
