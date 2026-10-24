import { useEffect, useState } from "react";
import { confirmDialog } from "../stores/confirmStore";
import { useFanOutStore } from "../stores/fanoutStore";
import { useToastStore } from "../stores/toastStore";
import { Chip } from "../ui/Chip";
import type { FanOut, FanOutCandidate } from "../types/domain";
import {
  verificationGlyph,
  verificationLabel,
  verificationTone,
} from "../app/v2/conversation/checkStatus";

function fmtTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

/// F9: the comparison. One row per candidate — what it changed, what it
/// ran and with which exit, the console's verdict on its words — and the
/// two acts the human can take: apply one (a --no-ff merge onto the base)
/// or discard them all. Nothing here ranks the candidates.
export function FanOutSection() {
  const items = useFanOutStore((s) => s.items);
  const error = useFanOutStore((s) => s.error);
  const load = useFanOutStore((s) => s.load);
  useEffect(() => {
    void load();
  }, [load]);
  const open = items.filter((f) => !f.closed);
  const closed = items.filter((f) => f.closed).slice(0, 3);
  if (items.length === 0 && !error) return null;
  return (
    <section className="rt-fanouts" data-testid="fanouts">
      <div className="wb-section-title">fan-outs</div>
      {error && <p className="wb-hint">{error}</p>}
      {open.map((f) => (
        <FanOutCard key={f.id} fo={f} />
      ))}
      {closed.map((f) => (
        <FanOutCard key={f.id} fo={f} />
      ))}
    </section>
  );
}

function FanOutCard({ fo }: { fo: FanOut }) {
  const apply = useFanOutStore((s) => s.apply);
  const discard = useFanOutStore((s) => s.discard);
  const showToast = useToastStore((s) => s.show);
  const [busy, setBusy] = useState(false);
  const done = fo.candidates.every((c) => c.status !== "running");
  const onApply = async (c: FanOutCandidate) => {
    const ok = await confirmDialog({
      title: `Apply ${c.participant.name}`,
      message: `Merge branch ${c.branch} onto ${fo.baseBranch} (--no-ff)? The other candidates' worktrees are removed; their branches stay.`,
      confirmLabel: "Apply",
    });
    if (!ok) return;
    setBusy(true);
    try {
      const out = await apply(fo.id, c.participant.id);
      showToast(
        out.merged
          ? `Applied ${c.participant.name} onto ${fo.baseBranch}`
          : `Not merged: ${out.detail || `${out.conflictFiles.length} conflicts`}`,
        out.merged ? "success" : "error",
      );
    } catch (e) {
      showToast(`Couldn't apply: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  const onDiscard = async () => {
    const ok = await confirmDialog({
      title: "Discard fan-out",
      message: "Remove every candidate's worktree and branch? Nothing is merged.",
      confirmLabel: "Discard",
      danger: true,
    });
    if (!ok) return;
    setBusy(true);
    try {
      await discard(fo.id);
    } catch (e) {
      showToast(`Couldn't discard: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="rt-fanout" data-testid={`fanout-${fo.id}`}>
      <div className="flex flex-wrap items-center gap-2 text-small">
        <span className="min-w-0 flex-1 truncate text-foreground" title={fo.problem}>
          {fo.problem}
        </span>
        <span className="font-code text-caption text-faint-foreground">→ {fo.baseBranch}</span>
        {fo.applied && (
          <Chip tone="ok" mono>
            applied {fo.candidates.find((c) => c.participant.id === fo.applied)?.participant.name}
          </Chip>
        )}
        {fo.closed && !fo.applied && (
          <Chip tone="neutral" mono>
            discarded
          </Chip>
        )}
        {!done && (
          <Chip tone="warn" mono>
            running
          </Chip>
        )}
      </div>
      <table className="rt-fanout-table">
        <thead>
          <tr>
            <th>agent</th>
            <th>diff</th>
            <th>checks</th>
            <th>verdict</th>
            <th>said</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {fo.candidates.map((c) => (
            <tr key={c.participant.id} data-testid={`fanout-candidate-${c.participant.id}`}>
              <td>
                <span className="text-foreground">{c.participant.name}</span>{" "}
                <span className="font-code text-caption text-faint-foreground">
                  {c.participant.engine ?? "claude"} {c.participant.model}
                </span>
              </td>
              <td className="font-code text-caption">
                {c.status === "running"
                  ? "…"
                  : c.status === "error"
                    ? `error: ${c.error ?? "?"}`
                    : `${c.files.length} file${c.files.length === 1 ? "" : "s"} +${c.insertions} −${c.deletions}`}
              </td>
              <td className="font-code text-caption">
                {c.commands === 0
                  ? "none"
                  : `${c.checks.passed}✓ ${c.checks.failed}✗${c.checks.unknown ? ` ${c.checks.unknown}?` : ""}`}
              </td>
              <td>
                <Chip tone={verificationTone(c.verification)} mono>
                  {verificationGlyph(c.verification)} {verificationLabel(c.verification)}
                </Chip>
              </td>
              <td
                className="max-w-[28ch] truncate text-caption text-muted-foreground"
                title={c.summary}
              >
                {c.summary || "—"}
                {c.tokens > 0 && (
                  <span className="font-code text-faint-foreground">
                    {" "}
                    · {fmtTokens(c.tokens)} tok
                  </span>
                )}
              </td>
              <td>
                {!fo.closed && (
                  <button
                    className="wb-cta"
                    disabled={busy || !done || c.status !== "done"}
                    onClick={() => void onApply(c)}
                    title={done ? "Merge this candidate onto the base" : "Wait for every candidate"}
                  >
                    apply
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {!fo.closed && (
        <div className="flex gap-2">
          <button className="wb-cta" disabled={busy} onClick={() => void onDiscard()}>
            discard all
          </button>
        </div>
      )}
    </div>
  );
}
