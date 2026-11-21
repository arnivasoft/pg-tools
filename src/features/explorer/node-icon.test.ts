import {
  Box, Columns3, Database, Eye, DatabaseZap, Folder, FunctionSquare, HardDrive, Hash, KeyRound, Layers, ListOrdered,
  Puzzle, Server, Shapes, Shield, ShieldCheck, SquareTerminal, Table2, TableProperties, User, Users, Zap,
} from "lucide-solid";
import { describe, expect, it, vi } from "vitest";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { iconFor } from "./node-icon";

// lucide-solid node ortamında yüklenemez (istemci API'si); her simge adını taşıyan ayrı bir nesneyle değiştirilir.
vi.mock("lucide-solid", () => {
  const names = [
    "Box", "Columns3", "Database", "Eye", "DatabaseZap", "Folder", "FunctionSquare", "HardDrive", "Hash", "KeyRound", "Layers", "ListOrdered", "Puzzle", "Server", "Shapes", "Shield", "ShieldCheck", "SquareTerminal", "Table2", "TableProperties", "User", "Users", "Zap",
  ];
  return Object.fromEntries(names.map((name) => [name, { name }]));
});

/** Tablo yeniden yazımından önceki `switch` (Plan 38 öncesi), birebir; davranış eşitliğinin referansı. */
function legacyIconFor(node: TreeNode) {
  switch (node.kind) {
    case "server":
      return Server;
    case "database":
      return Database;
    case "schema":
      return Layers;
    case "table":
      return node.flags.partitioned ? TableProperties : Table2;
    case "column":
      return node.flags.primaryKey ? KeyRound : Columns3;
    case "constraint":
      return node.flags.primaryKey ? KeyRound : ShieldCheck;
    case "index":
      return ListOrdered;
    case "trigger":
    case "triggerFunction":
      return Zap;
    case "policy":
      return Shield;
    case "view":
      return Eye;
    case "materializedView":
      return DatabaseZap;
    case "function":
      return FunctionSquare;
    case "procedure":
      return SquareTerminal;
    case "sequence":
      return Hash;
    case "type":
      return Shapes;
    case "domain":
      return Box;
    case "extension":
      return Puzzle;
    case "role":
      return node.detail === "group" ? Users : User;
    case "tablespace":
      return HardDrive;
    default:
      return Folder;
  }
}

/** `Record` her `NodeKind`'i zorunlu kılar; değer: Plan 38 türü mü. */
const KINDS: Record<NodeKind, boolean> = {
  serverGroup: false, server: false, databaseCollection: false, database: false, schemaCollection: false, schema: false,
  tableCollection: false, table: false, columnCollection: false, column: false, constraintCollection: false, constraint: false,
  indexCollection: false, index: false, triggerCollection: false, trigger: false, policyCollection: false, policy: false,
  partitionCollection: false, viewCollection: false, view: false, materializedViewCollection: false, materializedView: false,
  functionCollection: false, function: false, procedureCollection: false, procedure: false, triggerFunctionCollection: false,
  triggerFunction: false, sequenceCollection: false, sequence: false, typeCollection: false, type: false, domainCollection: false,
  domain: false, extensionCollection: false, extension: false, roleCollection: false, role: false, tablespaceCollection: false,
  tablespace: false,
  foreignDataWrapperCollection: true, foreignDataWrapper: true, foreignServerCollection: true, foreignServer: true,
  userMappingCollection: true, userMapping: true, foreignTableCollection: true, foreignTable: true,
  publicationCollection: true, publication: true, subscriptionCollection: true, subscription: true,
};

const base = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };

function variants(kind: NodeKind): TreeNode[] {
  const n = (flags: Partial<typeof base>, detail: string | null = null): TreeNode => ({
    id: "x", kind, label: "x", detail, hasChildren: false, serverId: null, flags: { ...base, ...flags }, color: null, tag: null,
  });
  return [n({}), n({ partitioned: true }), n({ primaryKey: true }), n({ connected: true }), n({}, "group"), n({}, "login")];
}

describe("iconFor (tür → simge tablosu)", () => {
  it("mevcut her tür, her bayrak/detay çeşitlemesinde yeniden yazımdan önceki simgeyi verir", () => {
    for (const [kind, isNew] of Object.entries(KINDS) as [NodeKind, boolean][]) {
      if (isNew) continue;
      for (const node of variants(kind)) expect([kind, iconFor(node)]).toEqual([kind, legacyIconFor(node)]);
    }
  });
  it("özel durumlar: partitioned tablo, PK kolon/kısıt, group rol", () => {
    const [plain, partitioned, pk, , group] = variants("table");
    expect(iconFor(plain)).toBe(Table2);
    expect(iconFor(partitioned)).toBe(TableProperties);
    expect(iconFor(pk)).toBe(Table2);
    expect(iconFor(variants("column")[2])).toBe(KeyRound);
    expect(iconFor(variants("constraint")[2])).toBe(KeyRound);
    expect(iconFor(variants("role")[4])).toBe(Users);
    expect(iconFor(variants("role")[0])).toBe(User);
    expect(iconFor(group)).toBe(Table2);
  });
  it("Plan 38: yeni türler Karar 6 simgelerini, koleksiyonları Folder'ı verir (bayraklardan bağımsız)", () => {
    const want: Partial<Record<NodeKind, unknown>> = {
      foreignDataWrapper: Puzzle, foreignServer: Server, userMapping: User, foreignTable: Table2, publication: DatabaseZap, subscription: Database,
    };
    for (const [kind, isNew] of Object.entries(KINDS) as [NodeKind, boolean][]) {
      if (!isNew) continue;
      for (const node of variants(kind)) expect([kind, iconFor(node)]).toEqual([kind, want[kind] ?? Folder]);
    }
  });
});
