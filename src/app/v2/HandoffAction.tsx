import type { useHandoff } from "./useHandoff";

/// The model picker the cards render next to their actions.
export function HandoffPicker({
  handoff,
  testid,
}: {
  handoff: ReturnType<typeof useHandoff>;
  testid: string;
}) {
  if (!handoff.available) {
    return (
      <p className="m-0 text-caption text-muted-foreground" data-testid={`${testid}-none`}>
        OpenCode is not installed: install it to continue on an open-weight model.
      </p>
    );
  }
  if (handoff.models.length === 0) {
    return (
      <p className="m-0 text-caption text-muted-foreground" data-testid={`${testid}-none`}>
        {handoff.loading
          ? "Reading OpenCode's models…"
          : "OpenCode has no models configured (opencode models)."}
      </p>
    );
  }
  return (
    <label className="flex items-center gap-2 text-caption text-muted-foreground">
      continue in OpenCode with
      <select
        aria-label="OpenCode model for the hand-over"
        data-testid={testid}
        className="rounded-1 border border-border bg-background px-1 text-caption text-foreground"
        value={handoff.chosen}
        onChange={(e) => handoff.setModel(e.target.value)}
      >
        {handoff.models.map((m) => (
          <option key={m.id} value={m.id}>
            {m.provider} · {m.name}
          </option>
        ))}
      </select>
    </label>
  );
}
