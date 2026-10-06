/// Which folder a session works in, relative to the open project: its isolated
/// worktree, else its own folder when that isn't the project checkout. Null =
/// the project root. Git, Changes, snapshots and the file tree follow this.

interface SessionPlace {
  cwd: string;
  worktree?: { path: string };
}

function trimSep(p: string): string {
  const t = p.replace(/[\\/]+$/, "");
  return t === "" ? p : t;
}

export function samePath(a: string, b: string): boolean {
  return trimSep(a) === trimSep(b);
}

export function sessionCheckout(
  session: SessionPlace | undefined,
  projectRoot: string,
): string | null {
  if (!session) return null;
  if (session.worktree?.path) return session.worktree.path;
  return samePath(session.cwd, projectRoot) ? null : session.cwd;
}

/// Name for the sidebar chip of a session that runs in a folder other than
/// the project (worktree sessions have their own ⎇ chip). Null otherwise.
export function sessionFolderName(
  session: SessionPlace | undefined,
  projectRoot: string,
): string | null {
  if (!session || session.worktree) return null;
  if (samePath(session.cwd, projectRoot)) return null;
  const parts = trimSep(session.cwd).split(/[\\/]/);
  return parts[parts.length - 1] || session.cwd;
}
