import { describe, expect, it, vi } from "vitest";
import { t } from "../../app/strings";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { IpcError } from "../../ipc/errors";
import { actionsFor, executeAction, qualifiedParts, type ActionDeps } from "./actions";
import { ROOT, emptyTree, type TreeState } from "./tree-model";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };

function sampleTree(): TreeState {
  const s = emptyTree();
  const add = (id: string, parent: string, kind: NodeKind, label: string, extra: Partial<TreeNode> = {}) => {
    s.nodes.set(id, { id, kind, label, detail: null, hasChildren: false, serverId: null, flags, color: null, tag: null, ...extra });
    s.parent.set(id, parent);
    s.children.set(parent, [...(s.children.get(parent) ?? []), id]);
  };
  add("srv-up", ROOT, "server", "up", { serverId: "u1", hasChildren: true, flags: { ...flags, connected: true } });
  add("srv-down", ROOT, "server", "down", { serverId: "d1", hasChildren: true });
  add("db-app", "srv-up", "database", "app", { hasChildren: true });
  add("sch-app", "db-app", "schema", "app", { hasChildren: true });
  add("coll-tables", "sch-app", "tableCollection", "Tables", { hasChildren: true });
  add("tbl-orders", "coll-tables", "table", "orders", { hasChildren: true });
  add("coll-cols", "tbl-orders", "columnCollection", "Columns", { hasChildren: true });
  add("col-id", "coll-cols", "column", "id");
  add("coll-idx", "tbl-orders", "indexCollection", "Indexes", { hasChildren: true });
  add("idx-1", "coll-idx", "index", "orders_pkey");
  add("coll-con", "tbl-orders", "constraintCollection", "Constraints", { hasChildren: true });
  add("con-1", "coll-con", "constraint", "orders_pk");
  add("coll-trg", "tbl-orders", "triggerCollection", "Triggers", { hasChildren: true });
  add("trg-1", "coll-trg", "trigger", "trg_audit");
  add("coll-part", "tbl-orders", "partitionCollection", "Partitions", { hasChildren: true });
  add("tbl-partition", "coll-part", "table", "orders_2024", { hasChildren: true });
  add("col-part", "tbl-partition", "column", "id");
  add("coll-fn", "sch-app", "functionCollection", "Functions", { hasChildren: true });
  add("fn-fmt-int", "coll-fn", "function", "fmt(integer)");
  return s;
}

const node = (s: TreeState, id: string) => s.nodes.get(id)!;

function deps(state: TreeState) {
  return {
    state: () => state,
    controller: {
      refresh: vi.fn(async () => {}),
      ensureConnected: vi.fn(async () => true),
      expand: vi.fn(async () => {}),
      disconnect: vi.fn(),
      serverRemoved: vi.fn(async () => {}),
      sessionLost: vi.fn(() => false),
      nodeGone: vi.fn(() => false),
    },
    api: { quoteIdentifiers: vi.fn(), getDdl: vi.fn(), deleteServer: vi.fn() },
    copy: vi.fn(async () => {}),
    notify: vi.fn(),
    editServer: vi.fn(),
    confirmDelete: vi.fn(async () => true),
  };
}
const asDeps = (d: ReturnType<typeof deps>) => d as unknown as ActionDeps;

describe("qualifiedParts / actionsFor", () => {
  const s = sampleTree();
  it("qualifiedParts follows the schema ancestor", () => {
    expect(qualifiedParts(s, "tbl-orders")).toEqual({ parts: ["app", "orders"], suffix: "" });
    expect(qualifiedParts(s, "col-id")).toEqual({ parts: ["app", "orders", "id"], suffix: "" });
    expect(qualifiedParts(s, "fn-fmt-int")).toEqual({ parts: ["app", "fmt"], suffix: "(integer)" });
    expect(qualifiedParts(s, "db-app")).toEqual({ parts: ["app"], suffix: "" });
  });
  it("hides qualified name where the schema is not trustworthy", () => {
    expect(qualifiedParts(s, "tbl-partition")).toBeNull();
    expect(qualifiedParts(s, "col-part")).toBeNull();
    expect(qualifiedParts(s, "idx-1")?.parts).toEqual(["app", "orders_pkey"]);
    expect(qualifiedParts(s, "con-1")).toBeNull();
    expect(qualifiedParts(s, "trg-1")).toBeNull();
  });
  it("menu per node kind", () => {
    expect(actionsFor(s, node(s, "srv-down"))).toEqual(["connect", "editServer", "deleteServer", "copyName"]);
    expect(actionsFor(s, node(s, "srv-up"))).toEqual(["refresh", "disconnect", "editServer", "deleteServer", "copyName"]);
    expect(actionsFor(s, node(s, "coll-tables"))).toEqual(["refresh", "copyName"]);
    expect(actionsFor(s, node(s, "tbl-orders"))).toEqual(["refresh", "copyName", "copyQualifiedName", "copyDdl"]);
    expect(actionsFor(s, node(s, "col-id"))).toEqual(["copyName", "copyQualifiedName", "copyDdl"]);
    expect(actionsFor(s, node(s, "con-1"))).toEqual(["copyName", "copyDdl"]);
  });
});

describe("executeAction", () => {
  it("action targets are re-read at execution time", async () => {
    const d = deps(emptyTree());
    await executeAction(asDeps(d), "copyName", "ghost");
    await executeAction(asDeps(d), "deleteServer", "ghost");
    expect(d.copy).not.toHaveBeenCalled();
    expect(d.api.deleteServer).not.toHaveBeenCalled();
    expect(d.notify).toHaveBeenCalledWith(t.menu.gone, "error");
  });

  it("copyQualifiedName quotes server-side and appends the signature", async () => {
    const d = deps(sampleTree());
    d.api.quoteIdentifiers.mockResolvedValue(["app", "fmt"]);
    await executeAction(asDeps(d), "copyQualifiedName", "fn-fmt-int");
    expect(d.api.quoteIdentifiers).toHaveBeenCalledWith("u1", ["app", "fmt"]);
    expect(d.copy).toHaveBeenCalledWith("app.fmt(integer)");
  });

  it("copyDdl reports an empty DDL instead of copying", async () => {
    const d = deps(sampleTree());
    d.api.getDdl.mockResolvedValue("");
    await executeAction(asDeps(d), "copyDdl", "tbl-orders");
    expect(d.copy).not.toHaveBeenCalled();
    expect(d.notify).toHaveBeenCalledWith(t.menu.noDdl, "info");
  });

  it("deleteServer does nothing when the confirmation is declined", async () => {
    const d = deps(sampleTree());
    d.confirmDelete.mockResolvedValue(false);
    await executeAction(asDeps(d), "deleteServer", "srv-up");
    expect(d.api.deleteServer).not.toHaveBeenCalled();
  });

  it("deleteServer removes the record, reloads the tree and toasts", async () => {
    const d = deps(sampleTree());
    d.api.deleteServer.mockResolvedValue(null);
    await executeAction(asDeps(d), "deleteServer", "srv-up");
    expect(d.api.deleteServer).toHaveBeenCalledWith("u1");
    expect(d.controller.serverRemoved).toHaveBeenCalledOnce();
    expect(d.notify).toHaveBeenCalledWith(t.menu.deleted("up"), "success");
  });

  it("deleteServer failure reports an error toast and reloads the server list", async () => {
    const d = deps(sampleTree());
    d.api.deleteServer.mockRejectedValue(new IpcError({ code: "config", message: "kayıt salt-okunur", detail: null, hint: null }));
    await executeAction(asDeps(d), "deleteServer", "srv-up");
    expect(d.notify).toHaveBeenCalledWith("kayıt salt-okunur", "error");
    expect(d.controller.serverRemoved).toHaveBeenCalledTimes(1);
  });

  it("backend not_found from a menu action marks the node gone", async () => {
    const d = deps(sampleTree());
    const gone = { code: "not_found", message: "yok", detail: null, hint: null };
    d.api.getDdl.mockRejectedValue(new IpcError(gone));
    await executeAction(asDeps(d), "copyDdl", "tbl-orders");
    expect(d.controller.nodeGone).toHaveBeenCalledWith("tbl-orders", expect.objectContaining({ code: "not_found" }));
    expect(d.notify).toHaveBeenCalledWith("yok", "error");
  });

  it("connect expands only after a successful connection", async () => {
    const d = deps(sampleTree());
    d.controller.ensureConnected.mockResolvedValue(false);
    await executeAction(asDeps(d), "connect", "srv-down");
    expect(d.controller.expand).not.toHaveBeenCalled();
  });
});

describe("deleteServer stale target", () => {
  it("re-reads the node after the confirmation dialog", async () => {
    const s = sampleTree();
    const d = deps(s);
    d.confirmDelete.mockImplementation(async () => {
      s.nodes.delete("srv-up");
      return true;
    });
    await executeAction(asDeps(d), "deleteServer", "srv-up");
    expect(d.api.deleteServer).not.toHaveBeenCalled();
    expect(d.notify).toHaveBeenCalledWith(t.menu.gone, "error");
  });
});
