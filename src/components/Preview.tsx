import { usePreviewStore } from "../stores/previewStore";
import { DiffViewer } from "./DiffViewer";

export function Preview() {
  const { selectedAbs, displayPath, mode, content, sizeBytes, truncated, loading, error } =
    usePreviewStore();

  if (!selectedAbs) {
    return (
      <div className="placeholder">
        Double-click a file in the tree to preview.{"\n"}
        Modified files open as diff; clean files open as content.
      </div>
    );
  }
  if (loading) return <div className="placeholder">Loading…</div>;
  if (error) {
    return (
      <div className="placeholder" style={{ color: "var(--danger)" }}>
        {error}
      </div>
    );
  }

  return (
    <div className="preview">
      <div className="preview-bar">
        <span className="preview-path">{displayPath}</span>
        {mode === "diff" && <span className="preview-meta">· modified · diff</span>}
        {mode === "content" && truncated && (
          <span className="preview-meta">· truncated to 1 MB</span>
        )}
        {mode === "content" && !truncated && (
          <span className="preview-meta">· {formatSize(sizeBytes)}</span>
        )}
        {mode === "binary" && (
          <span className="preview-meta">· binary · {formatSize(sizeBytes)}</span>
        )}
      </div>
      <div className="preview-body">
        {mode === "diff" && <DiffViewer diff={content} />}
        {mode === "content" && <pre className="preview-content">{content || " "}</pre>}
        {mode === "binary" && <div className="placeholder">Binary file — not previewable.</div>}
      </div>
    </div>
  );
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
