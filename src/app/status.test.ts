import { describe, expect, it } from "vitest";
import type { TreeNode } from "../ipc/bindings/TreeNode";
import { ROOT, emptyTree, setChildren, type TreeState } from "../features/explorer/tree-model";
import { statusText } from "./status";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
function node(id: string, kind: TreeNode["kind"], extra: Partial<TreeNode> = {}): TreeNode {
  return { id, kind, label: id, detail: null, hasChildren: true, serverId: null, flags, color: null, tag: null, ...extra };
}

function fixtureTree(): TreeState {
  let s = emptyTree();
  s = setChildren(s, ROOT, [
    node("srv-up", "server", { label: "pg-prod", detail: "16.4", serverId: "srv-up", flags: { ...flags, connected: true } }),
    node("srv-down", "server", { label: "pg-down", serverId: "srv-down" }),
  ]);
  s = setChildren(s, "srv-up", [node("db-app", "database", { label: "app", serverId: "srv-up" })]);
  s = setChildren(s, "db-app", [node("tbl-orders", "table", { label: "orders", serverId: "srv-up" })]);
  return s;
}

describe("statusText", () => {
  it.each([
    [null, "Hazır"],
    ["srv-up", "pg-prod · PostgreSQL 16.4 · bağlı"],
    ["db-app", "pg-prod · PostgreSQL 16.4 · app · bağlı"],
    ["tbl-orders", "pg-prod · PostgreSQL 16.4 · app · bağlı"],
    ["srv-down", "pg-down · bağlı değil"],
  ])("statusText(%s)", (selected, expected) => {
    expect(statusText({ ...fixtureTree(), selected })).toBe(expected);
  });
});
