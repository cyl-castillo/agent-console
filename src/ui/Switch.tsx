import { Switch as BaseSwitch } from "@base-ui/react/switch";
import { Checkbox as BaseCheckbox } from "@base-ui/react/checkbox";
import { Check } from "lucide-react";
import { cn } from "@/lib/cn";

export function Switch({
  checked,
  onCheckedChange,
  label,
  disabled,
  className,
}: {
  checked: boolean;
  onCheckedChange: (v: boolean) => void;
  /// Visible label; rendered as the control's accessible name.
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <label
      className={cn("inline-flex cursor-pointer items-center gap-2 font-sans text-body", className)}
    >
      <BaseSwitch.Root
        checked={checked}
        onCheckedChange={onCheckedChange}
        disabled={disabled}
        className={cn(
          "relative inline-flex h-4 w-7 shrink-0 rounded-pill border border-border bg-surface-3 p-px transition-colors dur-2",
          "data-checked:border-primary data-checked:bg-primary",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50",
        )}
      >
        <BaseSwitch.Thumb className="block size-3 rounded-pill bg-foreground transition-transform dur-2 data-checked:translate-x-3 data-checked:bg-background" />
      </BaseSwitch.Root>
      <span>{label}</span>
    </label>
  );
}

export function Checkbox({
  checked,
  onCheckedChange,
  label,
  disabled,
  className,
}: {
  checked: boolean;
  onCheckedChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <label
      className={cn("inline-flex cursor-pointer items-center gap-2 font-sans text-body", className)}
    >
      <BaseCheckbox.Root
        checked={checked}
        onCheckedChange={onCheckedChange}
        disabled={disabled}
        className={cn(
          "inline-flex size-4 shrink-0 items-center justify-center rounded-1 border border-border bg-background transition-colors dur-2",
          "data-checked:border-primary data-checked:bg-primary",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50",
        )}
      >
        <BaseCheckbox.Indicator className="text-background">
          <Check size={12} strokeWidth={3} />
        </BaseCheckbox.Indicator>
      </BaseCheckbox.Root>
      <span>{label}</span>
    </label>
  );
}
