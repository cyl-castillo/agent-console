import { useEffect, useState } from "react";

import { useAssistantStore } from "../stores/assistantStore";
import { Button, Chip, ConfirmDialog, EmptyState, Switch } from "../ui";

const KIND_LABEL: Record<string, string> = {
  product: "Product",
  guide: "Assistant's guide",
  tooling: "Missing tool",
  voice: "Voice",
};

/// The assistant's learning log: an opt-in, local record of what it was
/// asked and how it went, and the gaps the human's own engine reads in it.
/// Proposals are model output: rendered as plain text, never as markup.
export function AssistantLearning() {
  const remember = useAssistantStore((s) => s.remember);
  const learning = useAssistantStore((s) => s.learning);
  const proposing = useAssistantStore((s) => s.proposing);
  const error = useAssistantStore((s) => s.learningError);
  const setRemember = useAssistantStore((s) => s.setRemember);
  const refresh = useAssistantStore((s) => s.refreshLearning);
  const propose = useAssistantStore((s) => s.propose);
  const clearLog = useAssistantStore((s) => s.clearLog);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const entries = learning?.entries ?? 0;
  const set = learning?.lastProposals ?? null;

  return (
    <div className="flex flex-col gap-4 p-3 font-sans text-body" data-testid="assistant-learning">
      <section className="flex flex-col gap-2">
        <Switch
          checked={remember}
          onCheckedChange={setRemember}
          label="Keep a learning log of what I ask"
        />
        <p className="m-0 text-small text-muted-foreground">
          Off by default. When on, each question, the answer, the tools it used and your rating are
          saved on this machine, with secrets redacted. Nothing is sent anywhere and it is not part
          of the ledger. You can delete it at any time.
        </p>
      </section>

      <section className="flex flex-col gap-2" aria-label="What the log holds">
        <p className="m-0" data-testid="assistant-learning-counts">
          {entries === 0
            ? "The log is empty."
            : `${entries} turns · ${learning?.failed ?? 0} failed · ${learning?.ratedUp ?? 0} helpful · ${learning?.ratedDown ?? 0} not helpful`}
        </p>
        {learning?.path && entries > 0 && (
          <p className="m-0 break-all font-code text-caption text-muted-foreground">
            {learning.path}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="primary"
            disabled={entries === 0}
            loading={proposing}
            onClick={() => void propose()}
            data-testid="assistant-propose"
          >
            Propose improvements
          </Button>
          <Button
            variant="ghost"
            disabled={entries === 0 || proposing}
            onClick={() => setConfirming(true)}
            data-testid="assistant-log-clear"
          >
            Delete the log
          </Button>
          <span className="text-caption text-muted-foreground">
            Proposing runs one turn of your agent over the last turns of the log.
          </span>
        </div>
        {error && (
          <p className="m-0 text-small text-danger" role="alert">
            {error}
          </p>
        )}
      </section>

      <section className="flex flex-col gap-2" aria-label="Proposed improvements">
        {set && set.proposals.length === 0 && (
          <p className="m-0 text-small text-muted-foreground">
            The last review of {set.analyzed} turns found nothing worth proposing.
          </p>
        )}
        {!set && entries > 0 && (
          <EmptyState
            title="No proposals yet"
            description="Ask for them when the log has a few days of real use: they are only as good as what it holds."
          />
        )}
        {set && set.proposals.length > 0 && (
          <>
            <p className="m-0 text-caption text-muted-foreground">
              From {set.analyzed} turns, {new Date(set.at).toLocaleString()}. Proposals to read,
              nothing was changed or filed.
            </p>
            <ul className="m-0 flex list-none flex-col gap-2 p-0">
              {set.proposals.map((p, i) => (
                <li
                  key={`${p.kind}-${i}`}
                  className="rounded-2 border border-border px-3 py-2"
                  data-testid="assistant-proposal"
                >
                  <div className="mb-1 flex flex-wrap items-center gap-2">
                    <Chip>{KIND_LABEL[p.kind] ?? "Product"}</Chip>
                    <span className="font-medium">{p.title}</span>
                  </div>
                  <p className="m-0 whitespace-pre-wrap break-words text-small">{p.rationale}</p>
                  {p.evidence.length > 0 && (
                    <ul className="m-0 mt-1 list-disc pl-5 text-small text-muted-foreground">
                      {p.evidence.map((e, j) => (
                        <li key={j} className="break-words">
                          {e}
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          </>
        )}
      </section>

      <ConfirmDialog
        open={confirming}
        danger
        title="Delete the learning log?"
        message="This removes every logged turn and the proposals of this project from this machine. It cannot be undone."
        confirmLabel="Delete"
        onResolve={(yes) => {
          setConfirming(false);
          if (yes) void clearLog();
        }}
      />
    </div>
  );
}
