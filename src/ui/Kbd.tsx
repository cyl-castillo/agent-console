import { cn } from "@/lib/cn";

/// A key cap. Several in a row read as a chord ("Ctrl" "Enter").
export function Kbd({ children, className }: { children: string; className?: string }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-1 border border-border bg-surface-1 px-1",
        "font-code text-caption text-muted-foreground",
        className,
      )}
    >
      {children}
    </kbd>
  );
}
