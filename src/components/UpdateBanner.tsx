import { useUpdaterStore } from "../stores/updaterStore";

export function UpdateBanner() {
  const phase = useUpdaterStore((s) => s.phase);
  const info = useUpdaterStore((s) => s.info);
  const manualInfo = useUpdaterStore((s) => s.manualInfo);
  const error = useUpdaterStore((s) => s.error);
  const install = useUpdaterStore((s) => s.install);
  const openDownload = useUpdaterStore((s) => s.openDownload);
  const dismiss = useUpdaterStore((s) => s.dismiss);

  const visible =
    phase === "available" ||
    phase === "available-manual" ||
    phase === "installing" ||
    (phase === "error" && !!error);
  if (!visible) return null;

  return (
    <div className="update-banner" role="status">
      {phase === "available" && info && (
        <>
          <span className="update-banner-text">
            Update available · v{info.currentVersion} → <strong>v{info.version}</strong>
          </span>
          <span className="update-banner-actions">
            <button className="btn btn-primary" onClick={() => install()}>
              Install &amp; restart
            </button>
            <button onClick={dismiss}>Later</button>
          </span>
        </>
      )}
      {phase === "available-manual" && manualInfo && (
        <>
          <span className="update-banner-text">
            Update available · v{manualInfo.currentVersion} → <strong>v{manualInfo.version}</strong>
            <span className="update-banner-hint"> · download the new package manually</span>
          </span>
          <span className="update-banner-actions">
            <button className="btn btn-primary" onClick={() => openDownload()}>
              Download
            </button>
            <button onClick={dismiss}>Later</button>
          </span>
        </>
      )}
      {phase === "installing" && <span className="update-banner-text">Downloading update…</span>}
      {phase === "error" && (
        <>
          <span className="update-banner-text">Update failed: {error}</span>
          <span className="update-banner-actions">
            <button onClick={dismiss}>Dismiss</button>
          </span>
        </>
      )}
    </div>
  );
}
