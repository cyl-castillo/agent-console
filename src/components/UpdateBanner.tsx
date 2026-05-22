import { useUpdaterStore } from "../stores/updaterStore";

export function UpdateBanner() {
  const phase = useUpdaterStore((s) => s.phase);
  const info = useUpdaterStore((s) => s.info);
  const error = useUpdaterStore((s) => s.error);
  const install = useUpdaterStore((s) => s.install);
  const dismiss = useUpdaterStore((s) => s.dismiss);

  if (phase !== "available" && phase !== "installing" && phase !== "error") return null;
  if (phase === "error" && !error) return null;

  return (
    <div className="update-banner" role="status">
      {phase === "available" && info && (
        <>
          <span className="update-banner-text">
            Update available · v{info.currentVersion} → <strong>v{info.version}</strong>
          </span>
          <span className="update-banner-actions">
            <button className="primary" onClick={() => install()}>Install &amp; restart</button>
            <button onClick={dismiss}>Later</button>
          </span>
        </>
      )}
      {phase === "installing" && (
        <span className="update-banner-text">Downloading update…</span>
      )}
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
