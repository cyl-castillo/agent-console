import { useEffect, useMemo, useRef } from "react";

import type { ChatBlock, Task } from "../types/domain";
import { useChatStore } from "../stores/chatStore";
import { useTaskStore } from "../stores/taskStore";
import { useChangesStore } from "../stores/changesStore";
import { MarkdownText } from "./MarkdownText";
import { ModeSelector } from "./ModeSelector";
import { ConstraintsEditor } from "./ConstraintsEditor";

/// Console panel — operational runtime UI for directing the agent.
/// Renamed conceptually from "chat" → "console". Code paths kept under the
/// existing names to avoid a sweeping refactor; only labels/visuals change.
export function AgentChat() {
  const { blocks, sending, inputDraft, totalCost, approveAll, setDraft, send, reset, restoreSnapshot } =
    useChatStore();
  const tasks = useTaskStore((s) => s.tasks);
  const mode = useTaskStore((s) => s.mode);
  const setMode = useTaskStore((s) => s.setMode);
  const toggleHistory = useTaskStore((s) => s.toggleHistory);
  const branch = useChangesStore((s) => s.status?.branch ?? null);

  const listRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  const groups = useMemo(() => groupByTask(tasks, blocks), [tasks, blocks]);
  const sideBlocks = useMemo(
    () => blocks.filter((b) => b.taskId === "" && b.kind === "info"),
    [blocks],
  );

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
    <div className="console">
      <div className="console-header">
        <span className="console-meta">session</span>
        <span className={`console-mode mode-${mode}`}>{mode}</span>
        {branch && <span className="console-meta">· {branch}</span>}
        <span className="console-meta">· {tasks.length} task{tasks.length === 1 ? "" : "s"}</span>
        {totalCost > 0 && <span className="console-meta">· ${totalCost.toFixed(4)}</span>}
        {approveAll && <span className="console-meta console-warn">· auto-approve</span>}
        <span className="spacer" />
        <button className="console-action" onClick={toggleHistory} title="Session history">history</button>
        <button className="console-action" onClick={reset} disabled={tasks.length === 0 && !sending}>reset</button>
      </div>

      <div className="console-stream" ref={listRef}>
        {tasks.length === 0 && sideBlocks.length === 0 && (
          <div className="console-empty">
            Runtime ready. Issue a task or type /help for commands.
          </div>
        )}
        {sideBlocks.map((b) => <InfoLine key={b.id} content={(b as { content: string }).content} />)}
        {groups.map(({ task, items }) => (
          <TaskBlock key={task.id} task={task} blocks={items} onRestore={restoreSnapshot} />
        ))}
      </div>

      <div className="console-composer">
        <ModeSelector value={mode} onChange={setMode} disabled={sending} />
        <ConstraintsEditor />
      </div>

      <form className="console-input" onSubmit={onSubmit}>
        <span className={`prompt-mode mode-${mode}`}>{mode}</span>
        <span className="prompt-caret">&gt;</span>
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
          placeholder={sending ? "running…" : "type a task or /slash command (Shift+Enter for newline)"}
          disabled={sending}
          rows={2}
          spellCheck={false}
        />
        {sending
          ? <button type="button" onClick={reset} className="prompt-stop">stop</button>
          : <button type="submit" className="prompt-send" disabled={!inputDraft.trim()}>↵</button>
        }
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

function InfoLine({ content }: { content: string }) {
  return <pre className="console-info">{content}</pre>;
}

function TaskBlock({ task, blocks, onRestore }: {
  task: Task;
  blocks: ChatBlock[];
  onRestore: (sha: string, userBlockId: string) => void;
}) {
  const userBlock = blocks.find((b): b is Extract<ChatBlock, { kind: "user" }> => b.kind === "user");
  const others = blocks.filter((b) => b.kind !== "user");
  const isRunning = task.status === "running" || task.status === undefined;
  const duration = task.completedAtMs ? (task.completedAtMs - task.createdAtMs) / 1000 : null;
  const time = new Date(task.createdAtMs).toLocaleTimeString();

  return (
    <div className={`task task-${task.status ?? "running"}`}>
      <div className="task-head">
        <span className={`task-mode mode-${task.mode}`}>{task.mode}</span>
        <span className="task-prompt">{task.prompt}</span>
        <span className="task-time">{time}</span>
      </div>

      {task.constraints.length > 0 && (
        <div className="task-constraints-line">
          <span className="task-constraints-label">constraints:</span>
          {task.constraints.map((c, i) => (
            <span key={i} className="task-constraint">{c}{i < task.constraints.length - 1 ? " ·" : ""}</span>
          ))}
        </div>
      )}

      <div className="task-stream">
        {others.map((b) => <StreamLine key={b.id} block={b} />)}
        {isRunning && <div className="stream-pending">[processing]<span className="caret" /></div>}
      </div>

      <div className="task-foot">
        {isRunning ? (
          <span className="task-status running">[running]</span>
        ) : (
          <>
            <span className={`task-status ${task.status}`}>[{task.status}]</span>
            {duration !== null && <span>· {duration.toFixed(1)}s</span>}
            {task.costUsd != null && <span>· ${task.costUsd.toFixed(4)}</span>}
            <span className="task-aggs">
              {task.filesRead.length > 0 &&
                <span title={task.filesRead.join("\n")}>· {task.filesRead.length}r</span>}
              {task.filesModified.length > 0 &&
                <span title={task.filesModified.join("\n")}>· {task.filesModified.length}m</span>}
              {task.commandsExecuted.length > 0 &&
                <span title={task.commandsExecuted.join("\n")}>· {task.commandsExecuted.length}c</span>}
            </span>
          </>
        )}
        {userBlock?.snapshot && !userBlock.restored && (
          <button
            className="task-restore"
            title="Restore working tree to before this turn"
            onClick={() => {
              if (userBlock.snapshot && confirm("Restore to before this turn? Uncommitted changes will be lost.")) {
                onRestore(userBlock.snapshot.commitSha, userBlock.id);
              }
            }}
          >· ↶ restore</button>
        )}
        {userBlock?.restored && <span className="task-restored">· restored</span>}
      </div>
    </div>
  );
}

function StreamLine({ block }: { block: ChatBlock }) {
  switch (block.kind) {
    case "text":
      return (
        <div className="stream-line stream-text">
          <MarkdownText content={block.content} />
        </div>
      );
    case "thinking":
      return (
        <div className="stream-line stream-thinking">
          <span className="stream-label">[processing]</span> {block.content}
        </div>
      );
    case "info":
      return <pre className="console-info">{(block as { content: string }).content}</pre>;
    case "tool":
      return <ToolLine block={block} />;
    case "user":
      return null;
  }
}

function ToolLine({ block }: { block: Extract<ChatBlock, { kind: "tool" }> }) {
  const icon =
    block.status === "running" ? "▸" :
    block.status === "ok"      ? "✓" : "✗";
  return (
    <div className={`stream-line stream-op op-${block.status}`}>
      <span className="op-icon">{icon}</span>
      <span className="op-name">{block.name}</span>
      <span className="op-input">{formatInput(block.name, block.input)}</span>
      {block.summary && <span className="op-summary">— {block.summary}</span>}
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
