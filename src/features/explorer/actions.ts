import { t } from "../../app/strings";
import type { ToastKind } from "../../app/toast-model";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import type { ExplorerController } from "./controller";
import { ROOT, ancestorOfKind, isCollection, serverOf, type TreeState } from "./tree-model";
import { canOpenQueryTool } from "../query/open-kinds";
import { canOpenDashboard } from "../dashboard/open-kinds";
import { canOpenData } from "../data/open-kinds";
import { canCreateIn, canEditObject, isTableRowKind } from "../designer/ddl-kinds";

/** Kolon/kısıt düğümünün sahibi (koleksiyonlar atlanarak ilk ata) tablo mu (görünüm/MV kolonu ya da domain kısıtı değil). */
function ownedByTable(s: TreeState, id: string): boolean {
  let cur = s.parent.get(id);
  while (cur !== undefined && cur !== ROOT) {
    const kind = s.nodes.get(cur)?.kind;
    if (!kind) return false;
    if (!isCollection(kind)) return kind === "table";
    cur = s.parent.get(cur);
  }
  return false;
}

export type ActionId =
  | "refresh" | "connect" | "disconnect" | "editServer" | "deleteServer"
  | "copyName" | "copyQualifiedName" | "copyDdl" | "openQueryTool" | "openDashboard" | "openData"
  | "createObject" | "editObject" | "renameObject" | "dropObject";

/**
 * Nesne diyaloğu (designer) eylemleri: adı `Object` ile biten dört eylem (ana paket bütçesi için liste yerine sonek);
 * kip, adın `Object` öncesidir (`createObject` → `create`). Yeni bir eylem adı bu soneki kullanmamalıdır.
 */
export type DesignerOpenMode = "create" | "edit" | "rename" | "drop";

export interface ActionDeps {
  state: () => TreeState;
  controller: Pick<ExplorerController, "refresh" | "ensureConnected" | "expand" | "disconnect" | "serverRemoved" | "sessionLost" | "nodeGone">;
  api: Pick<Api, "quoteIdentifiers" | "getDdl" | "deleteServer">;
  copy(text: string): Promise<void>;
  notify(text: string, kind: ToastKind): void;
  editServer(serverId: string): void;
  confirmDelete(name: string): Promise<boolean>;
  /** "Query Tool'da aç" (sunucu/veritabanı/şema/tablo/görünüm). */
  openQueryTool?(id: string): void;
  /** "Dashboard'u aç" (bağlı sunucu / veritabanı). */
  openDashboard?(id: string): void;
  /** "Verileri Görüntüle/Düzenle" (table/view/MV; kilitli sunucuda da açılır — salt okunur). */
  openData?(id: string): void;
  /** Nesne diyaloğunu aç (Schema/Sequence/Extension/Index; `drop` sekme açmaz, silme diyaloğu). */
  openDesigner?(id: string, mode: DesignerOpenMode): void;
  /** Düğümün sunucusu salt okunur kilitli mi (designer eylemleri kapalı)? Verilmezse ağaçtaki sunucu düğümünün kilidi. */
  serverLocked?(nodeId: string): boolean;
  /** Sunucunun sorgu oturumlarını kapatacak işlemden (silme, bağlantı kesme) önce: açık transaction'lar için sorar. */
  beforeServerChange?(serverId: string, name: string): Promise<boolean>;
  /** İşlemden sonra: o sunucunun sorgu sekmeleri oturumsuz kalır (`gone`: kayıt silindi). */
  afterServerChange?(serverId: string, gone: boolean): void;
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
  const q: ActionId[] = canOpenQueryTool(node.kind) ? ["openQueryTool"] : [];
  if (canOpenData(node.kind)) q.push("openData");
  if (canOpenDashboard(node)) q.push("openDashboard");
  if (node.kind === "serverGroup") return ["refresh", "copyName"];
  if (node.kind === "server") {
    return node.flags.connected
      ? [...q, "refresh", "disconnect", "editServer", "deleteServer", "copyName"]
      : [...q, "connect", "editServer", "deleteServer", "copyName"];
  }
  if (isCollection(node.kind)) return canCreateIn(node.kind) ? ["createObject", "refresh", "copyName"] : ["refresh", "copyName"];
  const out: ActionId[] = [...q];
  // Önceden tanımlı pg_* rolleri arka uçta da reddedilir; menü göstermez.
  if (canEditObject(node.kind) && !(node.kind === "role" && node.flags.system)) out.push("editObject", "renameObject", "dropObject");
  else if (isTableRowKind(node.kind) && ownedByTable(s, node.id)) out.push("editObject");
  if (node.hasChildren) out.push("refresh");
  out.push("copyName");
  if (qualifiedParts(s, node.id) !== null) out.push("copyQualifiedName");
  out.push("copyDdl");
  return out;
}

/** Menüde ayırıcı yerleşimi için eylem grubu. */
export function groupOf(a: ActionId): number {
  if (a === "openQueryTool" || a === "openDashboard" || a === "openData") return -1;
  if (a.endsWith("Object")) return 3;
  if (a === "refresh" || a === "connect" || a === "disconnect") return 0;
  if (a === "editServer" || a === "deleteServer") return 1;
  return 2;
}

/** Eylem görünür ama kapalıysa gerekçesi (menüde `title`); designer eylemleri kilitli sunucuda kapalıdır. */
export function disabledReason(a: ActionId, deps: Pick<ActionDeps, "serverLocked" | "state">, nodeId: string): string | null {
  const locked = a.endsWith("Object") && (deps.serverLocked?.(nodeId) ?? serverOf(deps.state(), nodeId)?.flags.locked);
  return locked ? t.query.hint.locked : null;
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
      case "openQueryTool":
        deps.openQueryTool?.(id);
        return;
      case "openDashboard":
        deps.openDashboard?.(id);
        return;
      case "openData":
        deps.openData?.(id);
        return;
      case "createObject":
      case "editObject":
      case "renameObject":
      case "dropObject":
        // Savunma: menü kilitli sunucuda bu öğeleri gerekçesiyle zaten kapatır (açıcı da kaydı yeniden denetler).
        if (!disabledReason(action, deps, id)) deps.openDesigner?.(id, action.slice(0, -6) as DesignerOpenMode);
        return;
      case "disconnect":
        if (node.serverId && deps.beforeServerChange && !(await deps.beforeServerChange(node.serverId, node.label))) return;
        deps.controller.disconnect(id);
        if (node.serverId) deps.afterServerChange?.(node.serverId, false);
        deps.notify(t.menu.disconnected, "info");
        return;
      case "editServer":
        if (node.serverId) deps.editServer(node.serverId);
        return;
      case "deleteServer": {
        if (!node.serverId) return;
        if (!(await deps.confirmDelete(node.label))) return;
        if (deps.beforeServerChange && !(await deps.beforeServerChange(node.serverId, node.label))) return;
        // Diyalog açıkken düğüm kaybolmuş olabilir: yeniden oku.
        const still = deps.state().nodes.get(id);
        if (!still?.serverId) {
          deps.notify(t.menu.gone, "error");
          return;
        }
        await deps.api.deleteServer(still.serverId);
        deps.afterServerChange?.(still.serverId, true);
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
    if (!deps.controller.sessionLost(id, payload)) deps.controller.nodeGone(id, payload);
    deps.notify(payload.message, "error");
    // Silme yarıda kalmış olabilir (ör. kayıt silindi ama parola silinemedi): ağaç gerçek kaydı göstersin.
    if (action === "deleteServer") await deps.controller.serverRemoved().catch(() => {});
  }
}
