import { useMemo, useState } from "react";

import type { GitFileChange } from "../types/domain";
import { badgeFor } from "./changeBadge";

interface Props {
  changes: GitFileChange[];
  selected: string | null;
  onSelect: (path: string) => void;
  /// Per-file action — usually "stage" or "unstage". Returns synchronously,
  /// the parent runs the IPC call.
  fileAction: { label: string; title: string; run: (path: string) => void };
  /// Bulk variant for a whole folder.
  folderAction: { title: string; run: (paths: string[]) => void };
  onRevert: (path: string) => void;
}

interface TreeNode {
  name: string;
  /// Full path from repo root; only filled for leaves.
  fullPath?: string;
  change?: GitFileChange;
  children: Map<string, TreeNode>;
}

/// Hierarchical view of a list of changes. Folders are collapsible and
/// show the count of files plus aggregate diff stats when available.
export function ChangeTree({
  changes,
  selected,
  onSelect,
  fileAction,
  folderAction,
  onRevert,
}: Props) {
  const root = useMemo(() => buildTree(changes), [changes]);
  // Default: collapse everything except the path to the selected file.
  const [openDirs, setOpenDirs] = useState<Set<string>>(() => initialOpenDirs(root, selected));

  const toggle = (path: string) => {
    setOpenDirs((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  return (
    <ul className="change-tree">
      {renderChildren(
        root,
        "",
        openDirs,
        toggle,
        {
          selected,
          onSelect,
          fileAction,
          folderAction,
          onRevert,
        },
        0,
      )}
    </ul>
  );
}

function renderChildren(
  node: TreeNode,
  parentPath: string,
  openDirs: Set<string>,
  toggle: (p: string) => void,
  ctx: {
    selected: string | null;
    onSelect: (p: string) => void;
    fileAction: Props["fileAction"];
    folderAction: Props["folderAction"];
    onRevert: (p: string) => void;
  },
  depth: number,
): React.ReactNode {
  const entries = Array.from(node.children.values());
  // Sort: folders before files, then alphabetic.
  entries.sort((a, b) => {
    const aIsDir = a.children.size > 0 || !a.fullPath;
    const bIsDir = b.children.size > 0 || !b.fullPath;
    if (aIsDir !== bIsDir) return aIsDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  return entries.map((child) => {
    const isLeaf = !!child.fullPath && child.children.size === 0;
    const path = parentPath ? `${parentPath}/${child.name}` : child.name;

    if (isLeaf && child.change) {
      return (
        <FileRow
          key={path}
          change={child.change}
          depth={depth}
          active={ctx.selected === child.change.path}
          actionLabel={ctx.fileAction.label}
          actionTitle={ctx.fileAction.title}
          onClick={() => ctx.onSelect(child.change!.path)}
          onAction={() => ctx.fileAction.run(child.change!.path)}
          onRevert={() => ctx.onRevert(child.change!.path)}
        />
      );
    }

    const open = openDirs.has(path);
    const leaves = collectLeafPaths(child);
    return (
      <li key={path} className="ct-dir">
        <div
          className="ct-dir-row"
          style={{ paddingLeft: 8 + depth * 12 }}
          onClick={() => toggle(path)}
          title={path}
        >
          <span className="ct-caret">{open ? "▾" : "▸"}</span>
          <span className="ct-folder-icon">▮</span>
          <span className="ct-name">{child.name}</span>
          <span className="ct-count">{leaves.length}</span>
          <button
            className="ct-folder-action"
            title={ctx.folderAction.title}
            onClick={(e) => {
              e.stopPropagation();
              ctx.folderAction.run(leaves);
            }}
          >
            {ctx.fileAction.label}
          </button>
        </div>
        {open && (
          <ul className="ct-children">
            {renderChildren(child, path, openDirs, toggle, ctx, depth + 1)}
          </ul>
        )}
      </li>
    );
  });
}

function FileRow({
  change,
  depth,
  active,
  actionLabel,
  actionTitle,
  onClick,
  onAction,
  onRevert,
}: {
  change: GitFileChange;
  depth: number;
  active: boolean;
  actionLabel: string;
  actionTitle: string;
  onClick: () => void;
  onAction: () => void;
  onRevert: () => void;
}) {
  const badge = badgeFor(change);
  const name = change.path.split("/").pop() ?? change.path;
  return (
    <li
      className={`ct-file ${active ? "active" : ""}`}
      style={{ paddingLeft: 8 + depth * 12 }}
      onClick={onClick}
      title={change.path}
    >
      <button
        className="ct-file-action"
        onClick={(e) => {
          e.stopPropagation();
          onAction();
        }}
        title={actionTitle}
      >
        {actionLabel}
      </button>
      <span className={`change-badge ${badge.kind}`}>{badge.label}</span>
      <span className="ct-name">{name}</span>
      <button
        className="ct-file-revert"
        onClick={(e) => {
          e.stopPropagation();
          onRevert();
        }}
        title="Discard changes"
      >
        ↺
      </button>
    </li>
  );
}

function buildTree(changes: GitFileChange[]): TreeNode {
  const root: TreeNode = { name: "", children: new Map() };
  for (const change of changes) {
    const parts = change.path.split("/").filter(Boolean);
    let node = root;
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      let next = node.children.get(part);
      if (!next) {
        next = { name: part, children: new Map() };
        node.children.set(part, next);
      }
      node = next;
    }
    node.fullPath = change.path;
    node.change = change;
  }
  // Collapse single-child chains for compact rendering (a/b/c.ts -> a/b/c.ts).
  collapseSingleChains(root);
  return root;
}

function collapseSingleChains(node: TreeNode) {
  for (const [key, child] of Array.from(node.children.entries())) {
    while (child.children.size === 1 && !child.fullPath) {
      const [onlyKey, onlyChild] = Array.from(child.children.entries())[0];
      child.name = `${child.name}/${onlyKey}`;
      child.children = onlyChild.children;
      child.fullPath = onlyChild.fullPath;
      child.change = onlyChild.change;
    }
    node.children.set(key, child); // ensure map updated
    collapseSingleChains(child);
  }
}

function collectLeafPaths(node: TreeNode): string[] {
  const out: string[] = [];
  if (node.fullPath) out.push(node.fullPath);
  for (const c of node.children.values()) out.push(...collectLeafPaths(c));
  return out;
}

function initialOpenDirs(root: TreeNode, selected: string | null): Set<string> {
  const open = new Set<string>();
  // If <= 20 changes, open everything. Otherwise only the path to selected.
  let leafCount = 0;
  const count = (n: TreeNode) => {
    if (n.fullPath) leafCount++;
    for (const c of n.children.values()) count(c);
  };
  count(root);
  if (leafCount <= 20) {
    const walk = (n: TreeNode, prefix: string) => {
      for (const c of n.children.values()) {
        const path = prefix ? `${prefix}/${c.name}` : c.name;
        if (c.children.size > 0 || !c.fullPath) open.add(path);
        walk(c, path);
      }
    };
    walk(root, "");
    return open;
  }
  if (selected) {
    const parts = selected.split("/").slice(0, -1);
    let acc = "";
    for (const p of parts) {
      acc = acc ? `${acc}/${p}` : p;
      open.add(acc);
    }
  }
  return open;
}
