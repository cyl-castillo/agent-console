import { cn } from "@/lib/cn";
import { Badge } from "../../ui/Chip";
import { Tooltip } from "../../ui/Tooltip";
import { useShellStore } from "../../stores/shellStore";
import { DESTINATIONS, type Destination } from "./destinations";

/// The five destinations, as a vertical nav. Each is a real link-like button
/// with its label visible (no 8 px captions), aria-current on the active one,
/// and a count where there is something to act on.
export function Rail({ counts }: { counts: Partial<Record<Destination, number>> }) {
  const destination = useShellStore((s) => s.destination);
  const goTo = useShellStore((s) => s.goTo);
  return (
    <nav
      aria-label="Destinations"
      data-testid="rail"
      className="flex h-full w-14 shrink-0 flex-col items-stretch gap-0.5 border-r border-border bg-surface-1 px-1 py-2"
    >
      {DESTINATIONS.map((d) => {
        const active = d.key === destination;
        const count = counts[d.key] ?? 0;
        const Icon = d.icon;
        return (
          <Tooltip key={d.key} content={d.label} side="right">
            <button
              type="button"
              aria-current={active ? "page" : undefined}
              aria-label={count ? `${d.label} (${count})` : d.label}
              data-testid={`rail-${d.key}`}
              onClick={() => goTo(d.key)}
              className={cn(
                "relative flex h-12 flex-col items-center justify-center gap-0.5 rounded-2 font-sans text-caption font-medium text-muted-foreground transition-colors dur-1",
                "hover:bg-[var(--hover)] hover:text-foreground",
                "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
                active && "bg-[var(--accent-12)] text-[var(--accent-ink)]",
              )}
            >
              <Icon size={18} strokeWidth={1.75} aria-hidden />
              <span>{d.label}</span>
              {count > 0 && (
                <Badge count={count} className="absolute right-1 top-1 bg-warn text-background" />
              )}
            </button>
          </Tooltip>
        );
      })}
    </nav>
  );
}
