import type { NodeKind } from "../../ipc/bindings/NodeKind";

/** "Query Tool'da aç" menüsünün göründüğü düğümler (gezgin ve sorgu modülleri arasında döngüsüz paylaşılır). */
export const QUERY_TOOL_KINDS: readonly NodeKind[] = ["server", "database", "schema", "table", "view", "materializedView"];

export function canOpenQueryTool(kind: NodeKind): boolean {
  return QUERY_TOOL_KINDS.includes(kind);
}
