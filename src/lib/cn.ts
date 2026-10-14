import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/// Class composition for the primitives: clsx for conditionals, tailwind-merge
/// so a caller's `px-2` wins over a variant's `px-3` instead of both applying.
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
