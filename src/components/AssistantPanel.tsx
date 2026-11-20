import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";

import { profileFor } from "../agents/profiles";
import { ASSISTANT_MAX_CHARS, useAssistantStore } from "../stores/assistantStore";
import { useSessionStore } from "../stores/sessionStore";
import { useVoiceStore } from "../stores/voiceStore";
import { Button, Chip, EmptyState, Field, Spinner, Textarea } from "../ui";
import { MarkdownText } from "../ui/MarkdownText";
import { cn } from "@/lib/cn";

const EXAMPLES = [
  "Where do I see what the agent did in the last turn?",
  "Open the ledger",
  "¿Qué sesiones necesitan mi atención?",
];

/// The console's own assistant (experimental): ask how the app works or tell
/// it where to go, in plain language. It runs on the engine the project
/// already uses, with a read-and-navigate subset of the console's tools; what
/// it may do is enforced in Rust (`assistant_service`), this is a view.
export function AssistantPanel() {
  const project = useSessionStore((s) => s.project?.root ?? null);
  const messages = useAssistantStore((s) => s.messages);
  const status = useAssistantStore((s) => s.status);
  const activity = useAssistantStore((s) => s.activity);
  const engine = useAssistantStore((s) => s.engine);
  const toolsAttached = useAssistantStore((s) => s.toolsAttached);
  const tokens = useAssistantStore((s) => s.tokens);
  const ask = useAssistantStore((s) => s.ask);
  const reset = useAssistantStore((s) => s.reset);
  const [draft, setDraft] = useState("");
  const endRef = useRef<HTMLDivElement>(null);
  const working = status === "working";
  const voicePhase = useVoiceStore((s) => s.phase);
  const voiceOn = voicePhase !== "off" && voicePhase !== "loading";
  const listening = voicePhase === "listening";
  // Hold to talk: the take goes straight to the assistant, in the speaker's
  // own words — no command grammar in between.
  const talkStart = () => void useVoiceStore.getState().pttStart("assistant");
  const talkEnd = () => void useVoiceStore.getState().pttStop();
  const talkKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if ((e.key === " " || e.key === "Enter") && !e.repeat) {
      e.preventDefault();
      talkStart();
    }
  };
  const talkKeyUp = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      talkEnd();
    }
  };

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: "end" });
  }, [messages.length, working]);

  const send = (text: string) => {
    if (!text.trim() || working) return;
    setDraft("");
    void ask(text);
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    send(draft);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send(draft);
    }
  };

  if (!project) {
    return (
      <EmptyState
        title="Open a project first"
        description="The assistant works on the project that is open in the console."
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col font-sans text-body" data-testid="assistant-panel">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-caption text-muted-foreground">
        <Chip tone="purple">experimental</Chip>
        <span>
          Answers about the console and moves around it for you. It cannot edit files, type into a
          session or approve anything.
        </span>
        <span className="flex-1" />
        {engine && (
          <span data-testid="assistant-engine">
            on {profileFor(engine).label}
            {tokens > 0 ? ` · ${tokens.toLocaleString()} tokens` : ""}
          </span>
        )}
        {messages.length > 0 && (
          <Button size="sm" variant="ghost" onClick={reset} data-testid="assistant-reset">
            New conversation
          </Button>
        )}
      </div>

      <div
        className="min-h-0 flex-1 overflow-auto p-3"
        role="log"
        aria-label="Conversation with the assistant"
        aria-live="polite"
      >
        {messages.length === 0 && !working && (
          <EmptyState
            title="Ask the console"
            description="Each question runs one turn of your own agent, so it spends tokens from your plan like any other turn."
            action={
              <div className="flex flex-col items-stretch gap-1">
                {EXAMPLES.map((q) => (
                  <Button key={q} size="sm" onClick={() => send(q)}>
                    {q}
                  </Button>
                ))}
              </div>
            }
          />
        )}
        <ul className="m-0 flex list-none flex-col gap-3 p-0">
          {messages.map((m) => (
            <li
              key={m.id}
              data-testid={`assistant-message-${m.role}`}
              className={cn(
                "rounded-2 border px-3 py-2",
                m.role === "user" && "border-border bg-surface-2",
                m.role === "assistant" && "border-transparent",
                m.role === "error" && "border-danger text-danger",
              )}
            >
              <div className="mb-1 text-caption font-medium text-muted-foreground">
                {m.role === "user"
                  ? "You"
                  : m.role === "assistant"
                    ? "Assistant"
                    : "Could not answer"}
              </div>
              {m.role === "assistant" ? (
                <MarkdownText text={m.text} />
              ) : (
                <p className="m-0 whitespace-pre-wrap break-words">{m.text}</p>
              )}
            </li>
          ))}
        </ul>
        {working && (
          <p
            className="m-0 mt-3 flex items-center gap-2 text-small text-muted-foreground"
            role="status"
            data-testid="assistant-activity"
          >
            <Spinner />
            {activity ?? "Working…"}
          </p>
        )}
        {!toolsAttached && (
          <p className="m-0 mt-3 text-small text-[var(--warn-ink)]" role="status">
            This build has no hook bridge, so the assistant answered from its guide and could not
            look at or drive the console.
          </p>
        )}
        <div ref={endRef} />
      </div>

      <form onSubmit={submit} className="flex flex-col gap-2 border-t border-border p-3">
        <Field
          label="Ask or tell the assistant"
          description="Enter sends, Shift+Enter adds a line."
        >
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Take me to the permissions"
            maxLength={ASSISTANT_MAX_CHARS}
            rows={2}
            data-testid="assistant-input"
          />
        </Field>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="submit"
            variant="primary"
            disabled={!draft.trim()}
            loading={working}
            data-testid="assistant-send"
          >
            Send
          </Button>
          {voiceOn ? (
            <Button
              type="button"
              aria-pressed={listening}
              disabled={working || voicePhase === "transcribing"}
              onPointerDown={talkStart}
              onPointerUp={talkEnd}
              onPointerLeave={talkEnd}
              onPointerCancel={talkEnd}
              onKeyDown={talkKeyDown}
              onKeyUp={talkKeyUp}
              data-testid="assistant-talk"
            >
              {listening ? "Listening… release to send" : "Hold to talk"}
            </Button>
          ) : (
            <span className="text-caption text-muted-foreground">
              Turn voice on (Ctrl+Shift+V) to talk to it; it answers aloud.
            </span>
          )}
        </div>
      </form>
    </div>
  );
}
