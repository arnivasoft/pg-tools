import type { NodeKind } from "../../ipc/bindings/NodeKind";

const LABELS: Record<NodeKind, string> = {
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
};

export function kindLabel(kind: NodeKind): string {
  return LABELS[kind];
}
