import { ancestorOfKind, serverOf, type TreeState } from "../features/explorer/tree-model";
import { t } from "./strings";

export function statusText(s: TreeState): string {
  const server = serverOf(s, s.selected);
  if (!server) return t.status.ready;
  const parts = [server.label];
  if (server.flags.connected) {
    if (server.detail) parts.push(`${t.status.engine} ${server.detail}`);
    const db = ancestorOfKind(s, s.selected, ["database"]);
    if (db) parts.push(db.label);
    parts.push(t.status.connected);
  } else {
    parts.push(t.status.notConnected);
  }
  return parts.join(" · ");
}
