import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { TreeNode } from "../../ipc/bindings/TreeNode";

export const ROOT = "";

export interface TreeState {
  nodes: Map<string, TreeNode>;
  parent: Map<string, string>;
  children: Map<string, string[]>;
  errors: Map<string, ErrorPayload>;
  loading: Set<string>;
  expanded: Set<string>;
  selected: string | null;
}

export function emptyTree(): TreeState {
  return { nodes: new Map(), parent: new Map(), children: new Map(), errors: new Map(), loading: new Set(), expanded: new Set(), selected: null };
}

function clone(s: TreeState): TreeState {
  return {
    nodes: new Map(s.nodes),
    parent: new Map(s.parent),
    children: new Map(s.children),
    errors: new Map(s.errors),
    loading: new Set(s.loading),
    expanded: new Set(s.expanded),
    selected: s.selected,
  };
}

function removeChildrenMut(s: TreeState, id: string): void {
  for (const child of s.children.get(id) ?? []) removeSubtreeMut(s, child);
  s.children.delete(id);
}

function removeSubtreeMut(s: TreeState, id: string): void {
  removeChildrenMut(s, id);
  s.nodes.delete(id);
  s.parent.delete(id);
  s.errors.delete(id);
  s.loading.delete(id);
  s.expanded.delete(id);
  if (s.selected === id) s.selected = null;
}

/** `id`, `ancestor`'ın kendisi veya torunu mu? */
function isWithin(s: TreeState, id: string | null, ancestor: string): boolean {
  let cur: string | undefined = id ?? undefined;
  while (cur !== undefined && cur !== ROOT) {
    if (cur === ancestor) return true;
    cur = s.parent.get(cur);
  }
  return false;
}

export function startLoading(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.loading.add(id);
  next.errors.delete(id);
  return next;
}

export function stopLoading(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.loading.delete(id);
  return next;
}

export function setChildren(s: TreeState, parentId: string, list: TreeNode[]): TreeState {
  if (parentId !== ROOT && !s.nodes.has(parentId)) return s;
  const next = clone(s);
  let disconnectedSelection: string | null = null;
  const selectedBefore = s.selected;
  const keep = new Set(list.map((n) => n.id));
  for (const old of s.children.get(parentId) ?? []) {
    // Düğüm bu arada başka bir ebeveyne taşınmış olabilir (ör. sunucunun grubu değişti);
    // yalnızca hâlâ bu ebeveyne ait olanları sil.
    if (!keep.has(old) && next.parent.get(old) === parentId) removeSubtreeMut(next, old);
  }
  for (const n of list) {
    const previous = s.nodes.get(n.id);
    const previousParent = next.parent.get(n.id);
    if (previousParent !== undefined && previousParent !== parentId) {
      const siblings = next.children.get(previousParent);
      if (siblings) next.children.set(previousParent, siblings.filter((c) => c !== n.id));
    }
    if (previous?.flags.connected && !n.flags.connected) {
      removeChildrenMut(next, n.id);
      next.expanded.delete(n.id);
      next.loading.delete(n.id);
      next.errors.delete(n.id);
      if (selectedBefore !== null && selectedBefore !== n.id && isWithin(s, selectedBefore, n.id)) {
        disconnectedSelection = n.id;
      }
    }
    next.nodes.set(n.id, n);
    next.parent.set(n.id, parentId);
  }
  next.children.set(parentId, list.map((n) => n.id));
  next.loading.delete(parentId);
  next.errors.delete(parentId);
  if (selectedBefore !== null && next.selected === null) {
    next.selected = parentId === ROOT ? null : parentId;
  }
  if (disconnectedSelection !== null) next.selected = disconnectedSelection;
  return next;
}

export function setError(s: TreeState, id: string, error: ErrorPayload): TreeState {
  const next = clone(s);
  next.loading.delete(id);
  next.errors.set(id, error);
  return next;
}

export function patchNode(s: TreeState, id: string, patch: Partial<TreeNode>): TreeState {
  const current = s.nodes.get(id);
  if (!current) return s;
  const next = clone(s);
  next.nodes.set(id, { ...current, ...patch });
  return next;
}

export function expand(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.expanded.add(id);
  return next;
}

export function collapse(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.expanded.delete(id);
  if (s.selected !== null && s.selected !== id && isWithin(s, s.selected, id)) next.selected = id;
  return next;
}

export function select(s: TreeState, id: string | null): TreeState {
  return { ...s, selected: id };
}

/** Çocukları siler ve düğümü daraltır (yeniden yüklenmeleri gerekir). */
export function invalidate(s: TreeState, id: string): TreeState {
  const next = clone(s);
  const selectedBefore = s.selected;
  removeChildrenMut(next, id);
  next.expanded.delete(id);
  if (selectedBefore !== null && selectedBefore !== id && isWithin(s, selectedBefore, id)) next.selected = id;
  return next;
}

export interface VisibleRow {
  id: string;
  node: TreeNode;
  depth: number;
  expanded: boolean;
  loading: boolean;
  error: ErrorPayload | null;
  childCount: number | null;
  setSize: number;
  posInSet: number;
}

export function flatten(s: TreeState): VisibleRow[] {
  const out: VisibleRow[] = [];
  const walk = (parentId: string, depth: number) => {
    const ids = s.children.get(parentId);
    if (!ids) return;
    ids.forEach((id, index) => {
      const node = s.nodes.get(id);
      if (!node) return;
      const expanded = s.expanded.has(id);
      const kids = s.children.get(id);
      out.push({
        id,
        node,
        depth,
        expanded,
        loading: s.loading.has(id),
        error: s.errors.get(id) ?? null,
        childCount: kids ? kids.length : null,
        setSize: ids.length,
        posInSet: index + 1,
      });
      if (expanded) walk(id, depth + 1);
    });
  };
  walk(ROOT, 0);
  return out;
}

export type Intent = { select?: string; expand?: string; collapse?: string; refresh?: string };

export function keyIntent(s: TreeState, rows: VisibleRow[], key: string): Intent | null {
  if (rows.length === 0) return null;
  const index = s.selected === null ? -1 : rows.findIndex((r) => r.id === s.selected);
  if (index < 0) {
    return ["ArrowDown", "ArrowUp", "Home", "End"].includes(key) ? { select: rows[0].id } : null;
  }
  const row = rows[index];
  switch (key) {
    case "ArrowDown":
      return index + 1 < rows.length ? { select: rows[index + 1].id } : null;
    case "ArrowUp":
      return index > 0 ? { select: rows[index - 1].id } : null;
    case "Home":
      return { select: rows[0].id };
    case "End":
      return { select: rows[rows.length - 1].id };
    case "ArrowRight":
      if (!row.node.hasChildren) return null;
      if (!row.expanded) return { expand: row.id };
      return rows[index + 1]?.depth === row.depth + 1 ? { select: rows[index + 1].id } : null;
    case "ArrowLeft": {
      if (row.expanded) return { collapse: row.id };
      const parent = s.parent.get(row.id);
      return parent && parent !== ROOT ? { select: parent } : null;
    }
    case "Enter":
      if (!row.node.hasChildren) return null;
      return row.expanded ? { collapse: row.id } : { expand: row.id };
    case "F5":
      return { refresh: row.id };
    default:
      return null;
  }
}

export function isCollection(kind: NodeKind): boolean {
  return kind.endsWith("Collection");
}

const numberFormat = new Intl.NumberFormat("tr-TR");

export function rowLabel(row: VisibleRow): string {
  return isCollection(row.node.kind) && row.childCount !== null
    ? `${row.node.label} (${numberFormat.format(row.childCount)})`
    : row.node.label;
}

export function serverOf(s: TreeState, id: string | null): TreeNode | null {
  let current = id;
  while (current !== null && current !== undefined && current !== ROOT) {
    const node = s.nodes.get(current);
    if (!node) return null;
    if (node.kind === "server") return node;
    current = s.parent.get(current) ?? null;
  }
  return null;
}
