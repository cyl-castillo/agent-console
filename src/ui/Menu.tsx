import type { ReactElement, ReactNode } from "react";
import { Menu as BaseMenu } from "@base-ui/react/menu";
import { ContextMenu as BaseContextMenu } from "@base-ui/react/context-menu";
import { cn } from "@/lib/cn";

export interface MenuItemDef {
  label: string;
  onSelect: () => void;
  icon?: ReactNode;
  /// Destructive: red text.
  danger?: boolean;
  disabled?: boolean;
  /// Shown dimmed at the right (a key, a hint).
  hint?: string;
  /// Draw a separator above this item.
  separatorBefore?: boolean;
  /// Explains a disabled item (title).
  title?: string;
}

const POPUP =
  "min-w-48 rounded-2 border border-border bg-surface-3 py-1 font-sans text-body text-foreground shadow-e2 outline-none data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-1";
const ITEM =
  "flex cursor-default select-none items-center gap-2 px-3 py-1.5 text-small outline-none data-highlighted:bg-[var(--hover)] data-disabled:opacity-50";

function Items({ items }: { items: MenuItemDef[] }) {
  return (
    <>
      {items.map((it, i) => (
        <div key={i} role="presentation">
          {it.separatorBefore && <BaseMenu.Separator className="my-1 h-px bg-border" />}
          <BaseMenu.Item
            onClick={it.onSelect}
            disabled={it.disabled}
            title={it.title}
            className={cn(ITEM, it.danger && "text-danger")}
          >
            {it.icon && (
              <span
                aria-hidden
                className="inline-flex shrink-0 text-muted-foreground [&>svg]:size-3.5"
              >
                {it.icon}
              </span>
            )}
            <span className="flex-1">{it.label}</span>
            {it.hint && (
              <span className="font-code text-caption text-faint-foreground">{it.hint}</span>
            )}
          </BaseMenu.Item>
        </div>
      ))}
    </>
  );
}

/// A dropdown of actions behind a trigger ("…"). Arrow keys move, Enter
/// picks, Escape closes; focus returns to the trigger.
export function Menu({
  trigger,
  items,
  align = "end",
  side = "bottom",
  open,
  onOpenChange,
}: {
  trigger: ReactElement;
  items: MenuItemDef[];
  align?: "start" | "center" | "end";
  side?: "top" | "bottom" | "left" | "right";
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  return (
    <BaseMenu.Root open={open} onOpenChange={onOpenChange}>
      <BaseMenu.Trigger render={trigger} />
      <BaseMenu.Portal>
        <BaseMenu.Positioner side={side} align={align} sideOffset={4} className="z-50">
          <BaseMenu.Popup className={POPUP}>
            <Items items={items} />
          </BaseMenu.Popup>
        </BaseMenu.Positioner>
      </BaseMenu.Portal>
    </BaseMenu.Root>
  );
}

/// The same actions on right-click (or long press) over `children`.
export function ContextMenu({
  children,
  items,
  className,
  ...rest
}: {
  children: ReactNode;
  items: MenuItemDef[];
  className?: string;
} & Record<`data-${string}`, string | undefined>) {
  return (
    <BaseContextMenu.Root>
      <BaseContextMenu.Trigger className={className} {...rest}>
        {children}
      </BaseContextMenu.Trigger>
      <BaseContextMenu.Portal>
        <BaseContextMenu.Positioner className="z-50">
          <BaseContextMenu.Popup className={POPUP}>
            <Items items={items} />
          </BaseContextMenu.Popup>
        </BaseContextMenu.Positioner>
      </BaseContextMenu.Portal>
    </BaseContextMenu.Root>
  );
}
