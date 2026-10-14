import { Toast as BaseToast } from "@base-ui/react/toast";
import { X } from "lucide-react";
import { cn } from "@/lib/cn";
import { IconButton } from "./IconButton";

export type ToastTone = "info" | "success" | "error";

/// One manager for the whole app: stores and non-React code call
/// `toast.add(...)`; the viewport renders wherever ToastHost is mounted.
export const toastManager = BaseToast.createToastManager();

export function showToast(message: string, tone: ToastTone = "info") {
  toastManager.add({
    description: message,
    type: tone,
    // Info/success are transient; errors persist until dismissed.
    timeout: tone === "error" ? 0 : 2600,
  });
}

function ToastList() {
  const { toasts } = BaseToast.useToastManager();
  return (
    <>
      {toasts.map((t) => (
        <BaseToast.Root
          key={t.id}
          toast={t}
          className={cn(
            "pointer-events-auto flex w-[360px] max-w-[calc(100vw-32px)] items-start gap-2 rounded-2 border bg-surface-3 px-3 py-2 font-sans text-body text-foreground shadow-e2",
            "data-starting-style:translate-y-2 data-starting-style:opacity-0 data-ending-style:opacity-0 transition-[opacity,transform] dur-2 motion-reduce:transition-none",
            t.type === "error" && "border-danger",
            t.type === "success" && "border-ok",
            t.type !== "error" && t.type !== "success" && "border-border",
          )}
        >
          <BaseToast.Content className="min-w-0 flex-1">
            {/* The toast is a dialog; the title names it (visually hidden when it is just the tone). */}
            <BaseToast.Title className={cn("font-semibold", !t.title && "sr-only")}>
              {t.title ??
                { error: "Error", success: "Success", info: "Notice" }[t.type ?? "info"] ??
                "Notice"}
            </BaseToast.Title>
            <BaseToast.Description className="break-words">{t.description}</BaseToast.Description>
          </BaseToast.Content>
          <BaseToast.Close
            render={<IconButton label="Dismiss" icon={<X size={14} />} size="sm" />}
          />
        </BaseToast.Root>
      ))}
    </>
  );
}

/// Mount once at the app root. Errors use role="alert"; the rest are polite.
export function ToastHost() {
  return (
    <BaseToast.Provider toastManager={toastManager}>
      <BaseToast.Portal>
        <BaseToast.Viewport className="pointer-events-none fixed bottom-7 right-3 z-[60] flex flex-col gap-2">
          <ToastList />
        </BaseToast.Viewport>
      </BaseToast.Portal>
    </BaseToast.Provider>
  );
}
