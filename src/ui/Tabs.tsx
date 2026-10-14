import type { ReactNode } from "react";
import { Tabs as BaseTabs } from "@base-ui/react/tabs";
import { cn } from "@/lib/cn";
import { Badge } from "./Chip";

export interface TabItem<T extends string> {
  value: T;
  label: string;
  count?: number;
  hint?: ReactNode;
}

/// Real tablist/tab/tabpanel roles with arrow-key navigation (what the
/// 19 hand-rolled tab strips never had).
export function Tabs<T extends string>({
  value,
  onValueChange,
  items,
  label,
  children,
  className,
}: {
  value: T;
  onValueChange: (v: T) => void;
  items: TabItem<T>[];
  label: string;
  /// Panels: one per item, keyed by value; pass a map so only the active
  /// panel is mounted.
  children: Partial<Record<T, ReactNode>>;
  className?: string;
}) {
  return (
    <BaseTabs.Root
      value={value}
      onValueChange={(v) => onValueChange(v as T)}
      className={cn("flex min-h-0 flex-col", className)}
    >
      <BaseTabs.List
        aria-label={label}
        className="relative flex h-8 shrink-0 items-end gap-1 border-b border-border px-1"
      >
        {items.map((it) => (
          <BaseTabs.Tab
            key={it.value}
            value={it.value}
            className={cn(
              "relative inline-flex h-7 items-center gap-1.5 rounded-t-1 px-2 font-sans text-body text-muted-foreground transition-colors dur-1",
              "hover:text-foreground data-active:text-foreground",
              "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
            )}
          >
            {it.label}
            {typeof it.count === "number" && <Badge count={it.count} />}
            {it.hint}
          </BaseTabs.Tab>
        ))}
        <BaseTabs.Indicator className="absolute bottom-[-1px] left-[var(--active-tab-left)] h-0.5 w-[var(--active-tab-width)] bg-primary transition-[left,width] dur-2 motion-reduce:transition-none" />
      </BaseTabs.List>
      {items.map((it) => (
        <BaseTabs.Panel key={it.value} value={it.value} className="min-h-0 flex-1 outline-none">
          {children[it.value]}
        </BaseTabs.Panel>
      ))}
    </BaseTabs.Root>
  );
}
