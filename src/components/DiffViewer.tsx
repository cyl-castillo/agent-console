/// Renders a unified diff with simple +/-/@@ line coloring.
/// Plain text source-of-truth — no syntax highlighting in v0.

interface Props {
  diff: string;
  empty?: string;
}

export function DiffViewer({ diff, empty = "No changes." }: Props) {
  if (!diff.trim()) {
    return <div className="placeholder">{empty}</div>;
  }
  const lines = diff.split("\n");
  return (
    <pre className="diff">
      {lines.map((line, i) => (
        <div key={i} className={lineClass(line)}>
          {line || " "}
        </div>
      ))}
    </pre>
  );
}

function lineClass(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---")) return "diff-header";
  if (line.startsWith("@@")) return "diff-hunk";
  if (line.startsWith("diff --git") || line.startsWith("index ")) return "diff-meta";
  if (line.startsWith("+")) return "diff-add";
  if (line.startsWith("-")) return "diff-del";
  return "diff-ctx";
}
