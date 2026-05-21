import { useEffect, useRef } from "react";

import type { ChatBlock } from "../types/domain";
import { useChatStore } from "../stores/chatStore";
import { MarkdownText } from "./MarkdownText";

export function AgentChat() {
  const { blocks, sending, inputDraft, totalCost, approveAll, setDraft, send, reset, restoreSnapshot } =
    useChatStore();
  const listRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [blocks]);

  // Global Ctrl+K to focus the chat input.
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
        <button onClick={reset} disabled={blocks.length === 0 && !sending}>Reset</button>
      </div>

      <div className="agent-messages" ref={listRef}>
        {blocks.length === 0 && (
          <div className="placeholder">
            Direct the agent. It runs as `claude -p` inside this repo.
            Risky actions (Bash/Edit/Write) ask for your approval; safe reads pass through.
            Edits appear in the Changes tab; a snapshot is created before each turn.
          </div>
        )}
        {blocks.map((b) => (
          <Block key={b.id} block={b} onRestore={restoreSnapshot} />
        ))}
        {sending && <div className="msg-role" style={{ marginLeft: 4 }}>working… <span className="caret" /></div>}
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

function Block({ block, onRestore }: {
  block: ChatBlock;
  onRestore: (sha: string, userId: string) => void;
}) {
  switch (block.kind) {
    case "user":
      return (
        <div className="msg msg-user">
          <div className="msg-role">
            you
            {block.snapshot && !block.restored && (
              <button
                className="msg-restore"
                title="Restore working tree to before this turn"
                onClick={() => {
                  if (block.snapshot && confirm("Restore to before this turn? Uncommitted changes will be lost.")) {
                    onRestore(block.snapshot.commitSha, block.id);
                  }
                }}
              >
                ↶ restore
              </button>
            )}
            {block.restored && <span className="msg-restored">· restored</span>}
          </div>
          <div className="msg-body">{block.content}</div>
        </div>
      );
    case "text":
      return (
        <div className="msg msg-assistant">
          <div className="msg-role">claude</div>
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
  }
}

function ToolBlock({ block }: { block: Extract<ChatBlock, { kind: "tool" }> }) {
  const icon =
    block.status === "running" ? "▸" :
    block.status === "ok"      ? "✓" : "✗";
  const cls = `tool tool-${block.status}`;
  return (
    <div className={cls}>
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
