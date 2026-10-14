import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";
import { Spinner } from "./Spinner";

export const buttonVariants = cva(
  [
    "inline-flex items-center justify-center gap-1.5 whitespace-nowrap select-none",
    "rounded-2 font-sans font-medium transition-colors dur-2",
    "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
    "disabled:pointer-events-none disabled:opacity-50",
  ],
  {
    variants: {
      variant: {
        primary: "bg-primary text-background hover:brightness-110 active:brightness-95",
        secondary:
          "bg-surface-2 text-foreground border border-border hover:bg-[var(--hover)] hover:border-[var(--hover-border)]",
        ghost: "text-muted-foreground hover:text-foreground hover:bg-[var(--hover)]",
        danger: "bg-destructive text-background hover:brightness-110 active:brightness-95",
      },
      size: {
        sm: "h-6 px-2 text-small",
        md: "h-7 px-3 text-body",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  /// Shows a spinner and disables the control; the label stays for width.
  loading?: boolean;
  /// Leading icon (16 px Lucide) or a Kbd hint on the trailing side.
  icon?: ReactNode;
  trailing?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    className,
    variant,
    size,
    loading,
    icon,
    trailing,
    children,
    disabled,
    type = "button",
    ...rest
  },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={cn(buttonVariants({ variant, size }), className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? <Spinner size={14} /> : icon}
      {children}
      {trailing}
    </button>
  );
});
