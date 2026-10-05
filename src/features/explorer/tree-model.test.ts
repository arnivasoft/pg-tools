import { describe, expect, it } from "vitest";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { TreeState } from "./tree-model";
import {
  ROOT, ancestorOfKind, collapse, invalidate, emptyTree, expand, flatten, keyIntent, patchNode, rowLabel, select, serverOf, setChildren, setError, startLoading,
} from "./tree-model";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
function node(id: string, extra: Partial<TreeNode> = {}): TreeNode {
  return { id, kind: "table", label: id, detail: null, hasChildren: true, serverId: null, flags, color: null, tag: null, ...extra };
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

  it("server list reload keeps a still-connected server expanded with its children (label-only save)", () => {
    const up = { ...flags, connected: true };
    let s = setChildren(emptyTree(), ROOT, [node("srv", { kind: "server", serverId: "S", flags: up, tag: null })]);
    s = expand(s, "srv");
    s = setChildren(s, "srv", [node("dbs", { kind: "databaseCollection" })]);
    s = setChildren(s, ROOT, [node("srv", { kind: "server", serverId: "S", flags: up, tag: "Test" })]);
    expect(s.nodes.get("srv")?.tag).toBe("Test");
    expect(s.expanded.has("srv")).toBe(true);
    expect(flatten(s).map((r) => r.id)).toEqual(["srv", "dbs"]);
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

describe("tree-model selection and stale results", () => {
  it("invalidate moves selection inside the subtree to the invalidated node", () => {
    const s = invalidate(select(sample(), "a2"), "a");
    expect(s.children.has("a")).toBe(false);
    expect(s.selected).toBe("a");
  });

  it("collapse moves a hidden descendant selection to the collapsed node", () => {
    let s = expand(sample(), "a1");
    s = setChildren(s, "a1", [node("a1x")]);
    s = select(s, "a1x");
    expect(collapse(s, "a").selected).toBe("a");
    expect(collapse(select(s, "b"), "a").selected).toBe("b");
  });

  it("setChildren for a missing parent is a no-op", () => {
    const s = sample();
    const r = setChildren(s, "gone", [node("zz")]);
    expect(r).toBe(s);
    expect(r.nodes.has("zz")).toBe(false);
  });

  it("disconnect moves selection to the server and clears its loading/error", () => {
    const on = { ...flags, connected: true };
    let s = setChildren(emptyTree(), ROOT, [node("srv", { kind: "server", flags: on })]);
    s = expand(s, "srv");
    s = setChildren(s, "srv", [node("dbs")]);
    s = select(s, "dbs");
    s = startLoading(s, "srv");
    s = setChildren(s, ROOT, [node("srv", { kind: "server", flags })]);
    expect(s.selected).toBe("srv");
    expect(s.loading.has("srv")).toBe(false);
    expect(s.errors.has("srv")).toBe(false);
  });

  it("finds the server ancestor of a node", () => {
    let s = emptyTree();
    const on = { ...flags, connected: true };
    s = setChildren(s, ROOT, [node("grp:G", { kind: "serverGroup" })]);
    s = setChildren(s, "grp:G", [node("srv", { kind: "server", flags: on, detail: "18.0" })]);
    s = setChildren(s, "srv", [node("dbs", { kind: "databaseCollection" })]);
    s = setChildren(s, "dbs", [node("db", { kind: "database" })]);
    expect(serverOf(s, "db")?.id).toBe("srv");
    expect(serverOf(s, "srv")?.id).toBe("srv");
    expect(serverOf(s, "grp:G")).toBeNull();
    expect(serverOf(s, null)).toBeNull();
  });
});

describe("tree-model reparenting servers between groups", () => {
  const srv = (extra: Partial<TreeNode> = {}) => node("srv:X", { kind: "server", ...extra });
  function grouped() {
    let s = emptyTree();
    s = setChildren(s, ROOT, [node("grp:A", { kind: "serverGroup" }), node("grp:B", { kind: "serverGroup" })]);
    s = expand(s, "grp:A");
    s = expand(s, "grp:B");
    s = setChildren(s, "grp:A", [srv()]);
    s = setChildren(s, "grp:B", []);
    return s;
  }

  it("keeps a server moved from a loaded group to root (root reloaded first)", () => {
    let s = grouped();
    s = setChildren(s, ROOT, [node("grp:A", { kind: "serverGroup" }), node("grp:B", { kind: "serverGroup" }), srv()]);
    s = setChildren(s, "grp:A", []);
    s = setChildren(s, "grp:B", []);
    expect(s.nodes.has("srv:X")).toBe(true);
    expect(s.parent.get("srv:X")).toBe(ROOT);
    expect(flatten(s).map((r) => r.id)).toEqual(["grp:A", "grp:B", "srv:X"]);
  });

  it("re-parenting removes the id from the previous parent's list", () => {
    let s = grouped();
    s = setChildren(s, ROOT, [node("grp:A", { kind: "serverGroup" }), node("grp:B", { kind: "serverGroup" }), srv()]);
    expect(s.children.get("grp:A")).toEqual([]);
    expect(flatten(s).map((r) => r.id)).toEqual(["grp:A", "grp:B", "srv:X"]);
  });

  it("moves a server between two loaded groups (target loaded first)", () => {
    const on = { ...flags, connected: true };
    let s = grouped();
    s = setChildren(s, "grp:A", [srv({ flags: on })]);
    s = expand(s, "srv:X");
    s = setChildren(s, "srv:X", [node("dbs")]);
    s = setChildren(s, "grp:B", [srv({ flags: on })]);
    s = setChildren(s, "grp:A", []);
    expect(s.parent.get("srv:X")).toBe("grp:B");
    expect(s.children.get("srv:X")).toEqual(["dbs"]);
    expect(flatten(s).map((r) => r.id)).toEqual(["grp:A", "grp:B", "srv:X", "dbs"]);
  });

  it("moves a server between two loaded groups (source loaded first)", () => {
    let s = grouped();
    s = setChildren(s, "grp:A", []);
    s = setChildren(s, "grp:B", [srv()]);
    expect(s.parent.get("srv:X")).toBe("grp:B");
    expect(flatten(s).map((r) => r.id)).toEqual(["grp:A", "grp:B", "srv:X"]);
  });
});

const kn = (id: string, kind: TreeNode["kind"], label: string, extra: Partial<TreeNode> = {}): TreeNode => node(id, { kind, label, ...extra });

function chain(tableNodes: TreeNode[] | null) {
  let s = emptyTree();
  s = setChildren(s, ROOT, [kn("g", "serverGroup", "Üretim")]);
  s = setChildren(s, "g", [kn("s1", "server", "pg-prod", { flags: { ...flags, connected: true } })]);
  s = setChildren(s, "s1", [kn("dbs", "databaseCollection", "Databases")]);
  s = setChildren(s, "dbs", [kn("db1", "database", "app")]);
  s = setChildren(s, "db1", [kn("sch", "schemaCollection", "Schemas")]);
  s = setChildren(s, "sch", [kn("pub", "schema", "public")]);
  s = setChildren(s, "pub", [kn("tbls", "tableCollection", "Tables")]);
  if (tableNodes) s = setChildren(s, "tbls", tableNodes);
  for (const id of ["g", "s1", "dbs", "db1", "sch", "pub", "tbls"]) s = expand(s, id);
  return s;
}
const tree = (labels: string[]) => chain(labels.map((l) => kn(`tbl-${l}`, "table", l, { hasChildren: false })));
const treeWithUnloadedTables = () => chain(null);
const withFilter = (s: TreeState, q: string): TreeState => ({ ...s, filter: q });

describe("filterRows", () => {
  it("shows matching nodes with their ancestors, expanded, and hides non-matching siblings", () => {
    const s = withFilter(tree(["orders", "customers", "order_items"]), "ord");
    const rows = flatten(s);
    expect(rows.map((r) => r.node.label)).toEqual(["Üretim", "pg-prod", "Databases", "app", "Schemas", "public", "Tables", "orders", "order_items"]);
    expect(rows.slice(0, -2).every((r) => r.expanded)).toBe(true);
    expect(rows.at(-1)!.posInSet).toBe(2);
    expect(rows.at(-1)!.setSize).toBe(2);
  });
  it("shows loaded total in collection count while filtered", () => {
    const rows = flatten(withFilter(tree(["orders", "customers", "order_items"]), "ord"));
    expect(rowLabel(rows.find((r) => r.id === "tbls")!)).toBe("Tables (3)");
  });
  it("is case-insensitive", () => {
    expect(flatten(withFilter(tree(["Orders"]), "oRDERS")).map((r) => r.node.label)).toContain("Orders");
  });
  it("matches Turkish dotted capital İ against plain i", () => {
    const s = tree(["invoice", "İstanbul", "kitap"]);
    const labels = (q: string) => flatten(withFilter(s, q)).filter((r) => r.node.kind === "table").map((r) => r.node.label);
    expect(labels("İnv")).toEqual(["invoice"]);
    expect(labels("ist")).toEqual(["İstanbul"]);
    expect(labels("KİTAP")).toEqual(["kitap"]);
  });
  it("treats the query as a plain substring", () => {
    const s = tree(["a.b", "axb", "100%", "c(d)", "back\\slash", "ÇOK", "İstanbul"]);
    for (const [q, expected] of [["a.b", ["a.b"]], [".*", []], ["100%", ["100%"]], ["(d)", ["c(d)"]], ["\\", ["back\\slash"]], ["çok", ["ÇOK"]], ["İst", ["İstanbul"]]] as const) {
      const labels = flatten(withFilter(s, q)).filter((r) => r.node.kind === "table").map((r) => r.node.label);
      expect(labels, q).toEqual(expected);
    }
  });
  it("blank query does not filter and keeps normal expansion", () => {
    const base = tree(["a", "b"]);
    expect(flatten(withFilter(base, "   "))).toEqual(flatten(base));
  });
  it("only searches loaded nodes", () => {
    expect(flatten(withFilter(treeWithUnloadedTables(), "hidden"))).toEqual([]);
  });
  it("does not change expansion or selection", () => {
    const base = { ...tree(["a"]), selected: "tbl-a" };
    const filtered = withFilter(base, "zzz");
    expect(filtered.selected).toBe("tbl-a");
    expect(filtered.expanded).toEqual(base.expanded);
  });
  it("filters 10,000 loaded nodes in under 50 ms", () => {
    const s = withFilter(tree(Array.from({ length: 10000 }, (_, i) => `table_${i}`)), "table_99");
    const times = Array.from({ length: 5 }, () => {
      const t = performance.now();
      flatten(s);
      return performance.now() - t;
    });
    console.log(`filter 10k min ms: ${Math.min(...times).toFixed(2)}`);
    expect(Math.min(...times)).toBeLessThan(50);
  });
});

describe("keyIntent while filtered", () => {
  it("ArrowLeft goes to parent, ArrowRight descends, Enter does nothing", () => {
    const s = withFilter(tree(["orders"]), "ord");
    const rows = flatten(s);
    expect(keyIntent(select(s, "tbls"), rows, "ArrowLeft")).toEqual({ select: "pub" });
    expect(keyIntent(select(s, "tbls"), rows, "ArrowRight")).toEqual({ select: "tbl-orders" });
    expect(keyIntent(select(s, "tbls"), rows, "Enter")).toBeNull();
    expect(keyIntent(select(s, "tbl-orders"), rows, "ArrowLeft")).toEqual({ select: "tbls" });
  });
  it("ArrowRight on a matching node whose children are hidden does not expand", () => {
    const base = chain([kn("tbl-x", "table", "xorders", { hasChildren: true })]);
    const s = withFilter(base, "xorders");
    expect(keyIntent(select(s, "tbl-x"), flatten(s), "ArrowRight")).toBeNull();
  });
});

describe("ancestorOfKind", () => {
  it("finds the nearest matching ancestor or self, else null", () => {
    const s = tree(["a"]);
    expect(ancestorOfKind(s, "tbl-a", ["schema"])?.id).toBe("pub");
    expect(ancestorOfKind(s, "tbl-a", ["table"])?.id).toBe("tbl-a");
    expect(ancestorOfKind(s, "tbl-a", ["function"])).toBeNull();
    expect(ancestorOfKind(s, null, ["server"])).toBeNull();
  });
});
