import { describe, expect, it } from "vitest";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import {
  ROOT, collapse, emptyTree, expand, flatten, keyIntent, patchNode, rowLabel, select, setChildren, setError, startLoading,
} from "./tree-model";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false };
function node(id: string, extra: Partial<TreeNode> = {}): TreeNode {
  return { id, kind: "table", label: id, detail: null, hasChildren: true, serverId: null, flags, ...extra };
}

function sample() {
  let s = emptyTree();
  s = setChildren(s, ROOT, [node("a"), node("b")]);
  s = expand(s, "a");
  s = setChildren(s, "a", [node("a1"), node("a2", { hasChildren: false })]);
  return s;
}

describe("tree-model", () => {
  it("flattens expanded nodes with depth and aria positions", () => {
    const rows = flatten(sample());
    expect(rows.map((r) => [r.id, r.depth, r.posInSet, r.setSize])).toEqual([
      ["a", 0, 1, 2], ["a1", 1, 1, 2], ["a2", 1, 2, 2], ["b", 0, 2, 2],
    ]);
    expect(rows[0].expanded).toBe(true);
    expect(rows[0].childCount).toBe(2);
    expect(rows[3].childCount).toBeNull();
  });

  it("collapsed nodes hide children but keep them loaded", () => {
    const s = collapse(sample(), "a");
    expect(flatten(s).map((r) => r.id)).toEqual(["a", "b"]);
    expect(s.children.get("a")).toEqual(["a1", "a2"]);
  });

  it("reconciles refreshed children: removes gone subtrees, keeps expansion", () => {
    let s = sample();
    s = expand(s, "a1");
    s = setChildren(s, "a1", [node("a1x")]);
    s = select(s, "a2");
    s = setChildren(s, "a", [node("a1"), node("a3")]);
    expect(s.nodes.has("a2")).toBe(false);
    expect(s.expanded.has("a1")).toBe(true);
    expect(s.children.get("a1")).toEqual(["a1x"]);
    expect(s.selected).toBe("a");
  });

  it("drops subtree when a server becomes disconnected", () => {
    let s = emptyTree();
    const on = { ...flags, connected: true };
    s = setChildren(s, ROOT, [node("srv", { kind: "server", flags: on })]);
    s = expand(s, "srv");
    s = setChildren(s, "srv", [node("dbs")]);
    s = setChildren(s, ROOT, [node("srv", { kind: "server", flags })]);
    expect(s.children.has("srv")).toBe(false);
    expect(s.expanded.has("srv")).toBe(false);
    expect(s.nodes.has("dbs")).toBe(false);
  });

  it("tracks loading and errors per node", () => {
    let s = startLoading(sample(), "b");
    expect(flatten(s).find((r) => r.id === "b")!.loading).toBe(true);
    s = setError(s, "b", { code: "network", message: "x", detail: null, hint: null });
    const b = flatten(s).find((r) => r.id === "b")!;
    expect([b.loading, b.error?.code]).toEqual([false, "network"]);
    s = setChildren(s, "b", []);
    expect(flatten(s).find((r) => r.id === "b")!.error).toBeNull();
  });

  it("patches nodes immutably", () => {
    const s = sample();
    const p = patchNode(s, "a", { detail: "16.4" });
    expect(p.nodes.get("a")!.detail).toBe("16.4");
    expect(s.nodes.get("a")!.detail).toBeNull();
  });

  it("maps keys to intents", () => {
    const s0 = sample();
    const rows = flatten(s0);
    expect(keyIntent(s0, rows, "ArrowDown")).toEqual({ select: "a" });
    const s = select(s0, "a1");
    expect(keyIntent(s, rows, "ArrowDown")).toEqual({ select: "a2" });
    expect(keyIntent(s, rows, "ArrowUp")).toEqual({ select: "a" });
    expect(keyIntent(s, rows, "End")).toEqual({ select: "b" });
    expect(keyIntent(s, rows, "Home")).toEqual({ select: "a" });
    expect(keyIntent(s, rows, "ArrowRight")).toEqual({ expand: "a1" });
    expect(keyIntent(s, rows, "ArrowLeft")).toEqual({ select: "a" });
    expect(keyIntent(s, rows, "F5")).toEqual({ refresh: "a1" });
    const onA = select(s0, "a");
    expect(keyIntent(onA, rows, "ArrowRight")).toEqual({ select: "a1" });
    expect(keyIntent(onA, rows, "ArrowLeft")).toEqual({ collapse: "a" });
    expect(keyIntent(onA, rows, "Enter")).toEqual({ collapse: "a" });
    const onLeaf = select(s0, "a2");
    expect(keyIntent(onLeaf, rows, "ArrowRight")).toBeNull();
    expect(keyIntent(onLeaf, rows, "x")).toBeNull();
  });

  it("labels loaded collections with Turkish thousands separator", () => {
    let s = emptyTree();
    s = setChildren(s, ROOT, [node("t", { kind: "tableCollection", label: "Tables" })]);
    expect(rowLabel(flatten(s)[0])).toBe("Tables");
    s = setChildren(s, "t", Array.from({ length: 1243 }, (_, i) => node(`t${i}`)));
    expect(rowLabel(flatten(s)[0])).toBe("Tables (1.243)");
  });
});
