import { t } from "../../app/strings";
import type { ToastKind } from "../../app/toast-model";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import type { ExplorerController } from "./controller";
import { ROOT, ancestorOfKind, isCollection, serverOf, type TreeState } from "./tree-model";

export type ActionId =
  | "refresh" | "connect" | "disconnect" | "editServer" | "deleteServer"
  | "copyName" | "copyQualifiedName" | "copyDdl";

export interface ActionDeps {
  state: () => TreeState;
  controller: Pick<ExplorerController, "refresh" | "ensureConnected" | "expand" | "disconnect" | "serverRemoved" | "sessionLost">;
  api: Pick<Api, "quoteIdentifiers" | "getDdl" | "deleteServer">;
  copy(text: string): Promise<void>;
  notify(text: string, kind: ToastKind): void;
  editServer(serverId: string): void;
  confirmDelete(name: string): Promise<boolean>;
}

const SCHEMA_QUALIFIED = ["table", "view", "materializedView", "sequence", "type", "domain", "index"];
const FUNCTION_LIKE = ["function", "procedure", "triggerFunction"];
const SELF_ONLY = ["schema", "database", "role", "tablespace", "extension"];

/** Ata zincirinde `partitionCollection` var mı? (Partition başka şemada olabilir; şema atasına güvenilmez.) */
function underPartitions(s: TreeState, id: string): boolean {
  let cur = s.parent.get(id);
  while (cur !== undefined && cur !== ROOT) {
    if (s.nodes.get(cur)?.kind === "partitionCollection") return true;
    cur = s.parent.get(cur);
  }
  return false;
}

export function qualifiedParts(s: TreeState, id: string): { parts: string[]; suffix: string } | null {
  const node = s.nodes.get(id);
  if (!node) return null;
  const kind = node.kind;
  if (SELF_ONLY.includes(kind)) return { parts: [node.label], suffix: "" };
  const parent = s.parent.get(id) ?? null;
  const schema = ancestorOfKind(s, parent, ["schema"]);
  if (!schema) return null;
  if (SCHEMA_QUALIFIED.includes(kind)) {
    if (underPartitions(s, id)) return null;
    return { parts: [schema.label, node.label], suffix: "" };
  }
  if (FUNCTION_LIKE.includes(kind)) {
    const open = node.label.indexOf("(");
    if (open < 0) return { parts: [schema.label, node.label], suffix: "" };
    return { parts: [schema.label, node.label.slice(0, open)], suffix: node.label.slice(open) };
  }
  if (kind === "column") {
    const rel = ancestorOfKind(s, parent, ["table", "view", "materializedView"]);
    if (!rel || underPartitions(s, id)) return null;
    return { parts: [schema.label, rel.label, node.label], suffix: "" };
  }
  return null;
}

export function actionsFor(s: TreeState, node: TreeNode): ActionId[] {
  if (node.kind === "serverGroup") return ["refresh", "copyName"];
  if (node.kind === "server") {
    return node.flags.connected
      ? ["refresh", "disconnect", "editServer", "deleteServer", "copyName"]
      : ["connect", "editServer", "deleteServer", "copyName"];
  }
  if (isCollection(node.kind)) return ["refresh", "copyName"];
  const out: ActionId[] = [];
  if (node.hasChildren) out.push("refresh");
  out.push("copyName");
  if (qualifiedParts(s, node.id) !== null) out.push("copyQualifiedName");
  out.push("copyDdl");
  return out;
}

/** Menüde ayırıcı yerleşimi için eylem grubu. */
export function groupOf(a: ActionId): number {
  if (a === "refresh" || a === "connect" || a === "disconnect") return 0;
  if (a === "editServer" || a === "deleteServer") return 1;
  return 2;
}

export async function executeAction(deps: ActionDeps, action: ActionId, id: string): Promise<void> {
  const node = deps.state().nodes.get(id);
  if (!node) {
    deps.notify(t.menu.gone, "error");
    return;
  }
  const serverId = node.kind === "server" ? node.serverId : (serverOf(deps.state(), id)?.serverId ?? null);
  try {
    switch (action) {
      case "refresh":
        await deps.controller.refresh(id);
        return;
      case "connect":
        if (await deps.controller.ensureConnected(id)) await deps.controller.expand(id);
        return;
      case "disconnect":
        deps.controller.disconnect(id);
        deps.notify(t.menu.disconnected, "info");
        return;
      case "editServer":
        if (node.serverId) deps.editServer(node.serverId);
        return;
      case "deleteServer": {
        if (!node.serverId) return;
        if (!(await deps.confirmDelete(node.label))) return;
        // Diyalog açıkken düğüm kaybolmuş olabilir: yeniden oku.
        const still = deps.state().nodes.get(id);
        if (!still?.serverId) {
          deps.notify(t.menu.gone, "error");
          return;
        }
        await deps.api.deleteServer(still.serverId);
        await deps.controller.serverRemoved();
        deps.notify(t.menu.deleted(node.label), "success");
        return;
      }
      case "copyName":
        await deps.copy(node.label);
        deps.notify(t.menu.copied, "success");
        return;
      case "copyQualifiedName": {
        const q = qualifiedParts(deps.state(), id);
        if (!q || !serverId) return;
        const quoted = await deps.api.quoteIdentifiers(serverId, q.parts);
        await deps.copy(quoted.join(".") + q.suffix);
        deps.notify(t.menu.copied, "success");
        return;
      }
      case "copyDdl": {
        const ddl = await deps.api.getDdl(id);
        if (!ddl) {
          deps.notify(t.menu.noDdl, "info");
          return;
        }
        await deps.copy(ddl);
        deps.notify(t.menu.copied, "success");
        return;
      }
    }
  } catch (e) {
    const payload = toPayload(e);
    deps.controller.sessionLost(id, payload);
    deps.notify(payload.message, "error");
  }
}
