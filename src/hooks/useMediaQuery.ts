import { useEffect, useState } from "react";

/// Reactive `matchMedia`. False where there is no window (store tests).
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() =>
    typeof window !== "undefined" && "matchMedia" in window
      ? window.matchMedia(query).matches
      : false,
  );
  useEffect(() => {
    if (typeof window === "undefined" || !("matchMedia" in window)) return;
    const mql = window.matchMedia(query);
    const on = () => setMatches(mql.matches);
    on();
    mql.addEventListener("change", on);
    return () => mql.removeEventListener("change", on);
  }, [query]);
  return matches;
}

/// Below this width the list pane and the inspector collapse on their own
/// (UI-2026-10.md F2): the centre column is what matters on a narrow window.
export const NARROW_QUERY = "(max-width: 900px)";
