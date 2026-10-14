import { Select as BaseSelect } from "@base-ui/react/select";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/cn";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  description?: string;
}

export function Select<T extends string>({
  value,
  onValueChange,
  options,
  label,
  placeholder = "Choose…",
  className,
}: {
  value: T | null;
  onValueChange: (v: T) => void;
  options: SelectOption<T>[];
  /// Accessible name of the trigger (visible label goes in a Field).
  label: string;
  placeholder?: string;
  className?: string;
}) {
  return (
    <BaseSelect.Root value={value} onValueChange={(v) => v != null && onValueChange(v as T)}>
      <BaseSelect.Trigger
        aria-label={label}
        className={cn(
          "inline-flex h-7 min-w-32 items-center justify-between gap-2 rounded-2 border border-border bg-background px-2 font-sans text-body text-foreground",
          "hover:border-[var(--hover-border)] focus-visible:outline-2 focus-visible:outline-offset-[-1px] focus-visible:outline-ring data-popup-open:border-primary",
          className,
        )}
      >
        <BaseSelect.Value placeholder={placeholder} className="truncate" />
        <BaseSelect.Icon className="text-faint-foreground">
          <ChevronDown size={14} />
        </BaseSelect.Icon>
      </BaseSelect.Trigger>
      <BaseSelect.Portal>
        <BaseSelect.Positioner sideOffset={4} className="z-50">
          <BaseSelect.Popup className="min-w-[var(--anchor-width)] rounded-2 border border-border bg-surface-3 py-1 shadow-e2 data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-1">
            <BaseSelect.List>
              {options.map((o) => (
                <BaseSelect.Item
                  key={o.value}
                  value={o.value}
                  className="flex cursor-default items-center gap-2 px-2 py-1 font-sans text-body text-foreground outline-none data-highlighted:bg-[var(--hover)]"
                >
                  <span className="flex min-w-0 flex-1 flex-col">
                    <BaseSelect.ItemText className="truncate">{o.label}</BaseSelect.ItemText>
                    {o.description && (
                      <span className="truncate text-caption text-faint-foreground">
                        {o.description}
                      </span>
                    )}
                  </span>
                  <BaseSelect.ItemIndicator className="text-primary">
                    <Check size={12} />
                  </BaseSelect.ItemIndicator>
                </BaseSelect.Item>
              ))}
            </BaseSelect.List>
          </BaseSelect.Popup>
        </BaseSelect.Positioner>
      </BaseSelect.Portal>
    </BaseSelect.Root>
  );
}
