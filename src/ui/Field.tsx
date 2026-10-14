import {
  forwardRef,
  type InputHTMLAttributes,
  type ReactNode,
  type TextareaHTMLAttributes,
} from "react";
import { Field as BaseField } from "@base-ui/react/field";
import { cn } from "@/lib/cn";

const control = [
  "w-full rounded-2 border border-border bg-background px-2 font-sans text-body text-foreground",
  "placeholder:text-faint-foreground transition-colors dur-2",
  "hover:border-[var(--hover-border)] focus:border-primary focus:outline-2 focus:outline-offset-[-1px] focus:outline-ring",
  "disabled:opacity-50 aria-invalid:border-danger",
].join(" ");

/// Label + control + description/error, wired with real `for`/`aria-describedby`.
export function Field({
  label,
  description,
  error,
  children,
  className,
}: {
  label: string;
  description?: ReactNode;
  error?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <BaseField.Root
      invalid={Boolean(error)}
      className={cn("flex flex-col gap-1 font-sans", className)}
    >
      <BaseField.Label className="text-small font-medium text-muted-foreground">
        {label}
      </BaseField.Label>
      {children}
      {description && !error && (
        <BaseField.Description className="text-caption text-faint-foreground">
          {description}
        </BaseField.Description>
      )}
      {error && (
        <BaseField.Error match className="text-caption text-danger">
          {error}
        </BaseField.Error>
      )}
    </BaseField.Root>
  );
}

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  function Input({ className, ...rest }, ref) {
    return <BaseField.Control ref={ref} className={cn(control, "h-7", className)} {...rest} />;
  },
);

export const Textarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement>
>(function Textarea({ className, rows = 3, ...rest }, ref) {
  return (
    <BaseField.Control
      render={<textarea rows={rows} />}
      ref={ref}
      className={cn(control, "min-h-14 resize-y py-1.5 leading-normal", className)}
      // Field.Control is typed for <input>; the rendered element is a textarea.
      {...(rest as Record<string, unknown>)}
    />
  );
});
