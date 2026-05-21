import { useState } from "react";
import type { FileNode } from "../types/domain";
import { usePreviewStore } from "../stores/previewStore";
import { useUIStore } from "../stores/uiStore";

interface Props {
  root: FileNode;
}

export function FileTree({ root }: Props) {
  return (
    <div className="tree">
      {(root.children ?? []).map((child) => (
        <TreeNode key={child.path} node={child} depth={0} />
      ))}
    </div>
  );
}

function TreeNode({ node, depth }: { node: FileNode; depth: number }) {
  const [open, setOpen] = useState(depth < 1);
  const [selected, setSelected] = useState(false);
  const isDir = node.isDir;
  const indent = 8 + depth * 12;

  const onClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isDir) {
      setOpen((o) => !o);
    } else {
      // Single click = select, do not open Preview.
      setSelected(true);
    }
  };

  const onDoubleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isDir) return;
    usePreviewStore.getState().open(node.path);
    useUIStore.getState().setTab("preview");
  };

  return (
    <>
      <div
        className={`tree-node ${selected ? "selected" : ""}`}
        style={{ paddingLeft: indent }}
        onClick={onClick}
        onDoubleClick={onDoubleClick}
        title={isDir ? node.name : "Double-click to preview"}
      >
        <span className="icon">
          {isDir ? (open ? "▾" : "▸") : " "}
        </span>
        <span className={`name ${isDir ? "dir" : "file"}`}>{node.name}</span>
      </div>
      {isDir && open && node.children?.map((child) => (
        <TreeNode key={child.path} node={child} depth={depth + 1} />
      ))}
    </>
  );
}
