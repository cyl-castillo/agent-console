import type { GitFileChange } from "../types/domain";

export type ChangeKind =
  | "modified"
  | "added"
  | "deleted"
  | "renamed"
  | "untracked";

/// Single source of truth for the M/A/D/R/U badge shown next to a changed file.
/// Both the tree view (`ChangeTree`) and the flat list (`ChangesList`) derive
/// their badge from this, so the same file never shows a different letter or
/// colour depending on which view you're in.
///
/// git's porcelain status is two columns — staged (X) and worktree (Y). We
/// prefer the worktree column when it carries a status, else the staged one.
export function badgeFor(c: GitFileChange): { label: string; kind: ChangeKind } {
  if (c.untracked) return { label: "U", kind: "untracked" };
  const code = c.code ?? "";
  const x = code[0] ?? " ";
  const y = code[1] ?? " ";
  const ch = y !== " " ? y : x;
  switch (ch) {
    case "A":
      return { label: "A", kind: "added" };
    case "D":
      return { label: "D", kind: "deleted" };
    case "R":
      return { label: "R", kind: "renamed" };
    case "M":
    default:
      return { label: "M", kind: "modified" };
  }
}
