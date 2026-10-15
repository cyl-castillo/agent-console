import { useEffect, useState } from "react";
import { ipc } from "../ipc/tauri";

/// Build provenance chip data, dev builds only. One fetch per mount; the
/// answer never changes for a running binary — that's the point: if the chip
/// doesn't match what you just edited, you're staring at a stale binary.
export function useBuildInfo(): { commit: string; builtAt: string; debug: boolean } | null {
  const [info, setInfo] = useState<{ commit: string; builtAt: string; debug: boolean } | null>(
    null,
  );
  useEffect(() => {
    ipc
      .appBuildInfo()
      .then((b) =>
        setInfo({
          commit: b.commit,
          builtAt: b.buildTimeSecs
            ? new Date(b.buildTimeSecs * 1000).toLocaleTimeString(undefined, {
                hour: "2-digit",
                minute: "2-digit",
              })
            : "?",
          debug: b.debug,
        }),
      )
      .catch(() => setInfo(null));
  }, []);
  return info;
}
