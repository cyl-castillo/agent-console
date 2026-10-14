import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// tailwind-merge only knows Tailwind's stock theme. Teach it ours, or
// `text-body` (a size) silently drops `text-background` (a colour) as a
// "conflict" and a primary button ends up with the default text colour.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      color: [
        "bg",
        "bg-elev-1",
        "bg-elev-2",
        "bg-elev-3",
        "border",
        "text",
        "text-dim",
        "text-muted",
        "text-faint",
        "accent",
        "danger",
        "ok",
        "warn",
        "purple",
        "background",
        "foreground",
        "surface-1",
        "surface-2",
        "surface-3",
        "muted-foreground",
        "faint-foreground",
        "primary",
        "destructive",
        "success",
        "warning",
        "ring",
      ],
      text: ["caption", "small", "body", "lead", "title", "display"],
      font: ["sans", "code"],
      shadow: ["e1", "e2", "e3"],
      radius: ["1", "2", "3", "pill"],
    },
  },
});

/// Class composition for the primitives: clsx for conditionals, tailwind-merge
/// so a caller's `px-2` wins over a variant's `px-3` instead of both applying.
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
