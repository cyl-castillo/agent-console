import type { ReactNode } from "react";
import { Command as Cmdk } from "cmdk";
import { cn } from "@/lib/cn";
import { Kbd } from "./Kbd";

export interface CommandItem {
  id: string;
  label: string;
  hint?: string;
  keywords?: string[];
  shortcut?: string[];
  group?: string;
  icon?: ReactNode;
  run: () => void;
}

/// Command palette body (cmdk): filterable list with groups, hints and
/// shortcuts. The host decides when it opens; this owns search and
/// keyboard selection and announces results to screen readers.
export function Command({
  items,
  placeholder = "Search actions…",
  value,
  onValueChange,
  emptyText = "No matches",
  className,
}: {
  items: CommandItem[];
  placeholder?: string;
  value?: string;
  onValueChange?: (v: string) => void;
  emptyText?: string;
  className?: string;
}) {
  const groups = new Map<string, CommandItem[]>();
  for (const it of items) {
    const g = it.group ?? "";
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g)!.push(it);
  }
  return (
    <Cmdk
      label="Command palette"
      className={cn("flex max-h-[60vh] flex-col overflow-hidden font-sans text-body", className)}
    >
      <Cmdk.Input
        value={value}
        onValueChange={onValueChange}
        placeholder={placeholder}
        className="h-10 w-full border-b border-border bg-transparent px-3 text-lead text-foreground outline-none placeholder:text-faint-foreground"
      />
      <Cmdk.List className="min-h-0 flex-1 overflow-auto py-1">
        <Cmdk.Empty className="px-3 py-6 text-center text-muted-foreground">{emptyText}</Cmdk.Empty>
        {[...groups.entries()].map(([group, list]) => (
          <Cmdk.Group
            key={group || "_"}
            heading={group || undefined}
            className="[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-caption [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-faint-foreground"
          >
            {list.map((it) => (
              <Cmdk.Item
                key={it.id}
                value={it.id}
                keywords={[it.label, ...(it.keywords ?? [])]}
                onSelect={it.run}
                className="group flex h-8 cursor-default items-center gap-2 px-3 text-foreground data-[selected=true]:bg-[var(--accent-12)]"
              >
                {it.icon && (
                  <span className="flex w-4 justify-center text-muted-foreground">{it.icon}</span>
                )}
                <span className="truncate">{it.label}</span>
                {it.hint && (
                  <span className="truncate text-small text-muted-foreground group-data-[selected=true]:text-foreground">
                    {it.hint}
                  </span>
                )}
                {it.shortcut && (
                  <span className="ml-auto flex gap-1">
                    {it.shortcut.map((k) => (
                      <Kbd key={k}>{k}</Kbd>
                    ))}
                  </span>
                )}
              </Cmdk.Item>
            ))}
          </Cmdk.Group>
        ))}
      </Cmdk.List>
    </Cmdk>
  );
}
