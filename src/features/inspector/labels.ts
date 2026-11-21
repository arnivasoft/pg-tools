import type { NodeKind } from "../../ipc/bindings/NodeKind";

type Collection = Extract<NodeKind, `${string}Collection`>;
type Item = Exclude<NodeKind, Collection>;

const LABELS: Record<Item, string> = {
  serverGroup: "Server Group",
  server: "Server",
  database: "Database",
  schema: "Schema",
  table: "Table",
  column: "Column",
  constraint: "Constraint",
  index: "Index",
  trigger: "Trigger",
  policy: "RLS Policy",
  view: "View",
  materializedView: "Materialized View",
  function: "Function",
  procedure: "Procedure",
  triggerFunction: "Trigger Function",
  sequence: "Sequence",
  type: "Type",
  domain: "Domain",
  extension: "Extension",
  role: "Role",
  tablespace: "Tablespace",
  foreignDataWrapper: "Foreign Data Wrapper",
  foreignServer: "Foreign Server",
  userMapping: "User Mapping",
  foreignTable: "Foreign Table",
  publication: "Publication",
  subscription: "Subscription",
};

// Plan 38 (ana paket telafisi): koleksiyon etiketi tekil etiketin sonuna "s" eklenerek türetilir; yalnız düzensiz çoğullar
// burada. Tekil türü olmayan koleksiyonlar (partitionCollection) tür denetimiyle zorunludur.
const PLURAL: Partial<Record<Collection, string>> & Record<Exclude<Collection, `${Item}Collection`>, string> = {
  indexCollection: "Indexes",
  policyCollection: "RLS Policies",
  partitionCollection: "Partitions",
  roleCollection: "Login/Group Roles",
};

export function kindLabel(kind: NodeKind): string {
  return kind.endsWith("Collection")
    ? PLURAL[kind as Collection] ?? `${LABELS[kind.slice(0, -10) as Item]}s`
    : LABELS[kind as Item];
}
