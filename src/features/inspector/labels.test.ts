import { describe, expect, it } from "vitest";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import { kindLabel } from "./labels";

/** Türetmeden önceki tam etiket tablosu (Plan 38 öncesi + 12 yeni tür); `Record` her türü zorunlu kılar. */
const EXPECTED: Record<NodeKind, string> = {
  serverGroup: "Server Group",
  server: "Server",
  databaseCollection: "Databases",
  database: "Database",
  schemaCollection: "Schemas",
  schema: "Schema",
  tableCollection: "Tables",
  table: "Table",
  columnCollection: "Columns",
  column: "Column",
  constraintCollection: "Constraints",
  constraint: "Constraint",
  indexCollection: "Indexes",
  index: "Index",
  triggerCollection: "Triggers",
  trigger: "Trigger",
  policyCollection: "RLS Policies",
  policy: "RLS Policy",
  partitionCollection: "Partitions",
  viewCollection: "Views",
  view: "View",
  materializedViewCollection: "Materialized Views",
  materializedView: "Materialized View",
  functionCollection: "Functions",
  function: "Function",
  procedureCollection: "Procedures",
  procedure: "Procedure",
  triggerFunctionCollection: "Trigger Functions",
  triggerFunction: "Trigger Function",
  sequenceCollection: "Sequences",
  sequence: "Sequence",
  typeCollection: "Types",
  type: "Type",
  domainCollection: "Domains",
  domain: "Domain",
  extensionCollection: "Extensions",
  extension: "Extension",
  roleCollection: "Login/Group Roles",
  role: "Role",
  tablespaceCollection: "Tablespaces",
  tablespace: "Tablespace",
  foreignDataWrapperCollection: "Foreign Data Wrappers",
  foreignDataWrapper: "Foreign Data Wrapper",
  foreignServerCollection: "Foreign Servers",
  foreignServer: "Foreign Server",
  userMappingCollection: "User Mappings",
  userMapping: "User Mapping",
  foreignTableCollection: "Foreign Tables",
  foreignTable: "Foreign Table",
  publicationCollection: "Publications",
  publication: "Publication",
  subscriptionCollection: "Subscriptions",
  subscription: "Subscription",
};

describe("kindLabel", () => {
  it("uses PostgreSQL terms", () => {
    expect(kindLabel("table")).toBe("Table");
    expect(kindLabel("serverGroup")).toBe("Server Group");
    expect(kindLabel("tableCollection")).toBe("Tables");
    expect(kindLabel("columnCollection")).toBe("Columns");
    expect(kindLabel("database")).toBe("Database");
    expect(kindLabel("policyCollection")).toBe("RLS Policies");
    expect(kindLabel("triggerFunctionCollection")).toBe("Trigger Functions");
    expect(kindLabel("materializedView")).toBe("Materialized View");
    expect(kindLabel("roleCollection")).toBe("Login/Group Roles");
  });
  it("her tür için türetmeden önceki etiketin aynısı (yeni FDW/replikasyon türleri dahil)", () => {
    for (const [kind, label] of Object.entries(EXPECTED)) expect([kind, kindLabel(kind as NodeKind)]).toEqual([kind, label]);
  });
});
