// Nesne diyaloğunun (designer) uygulamaya giriş noktası (tembel parça): denetleyiciyi kurar ve gezgin düğümünden açma
// akışını taşır. Ana paket bu modülü yalnızca ilk "Oluştur/Düzenle/Yeniden adlandır/Sil"de dinamik içe aktarır.
//
// Kurallar:
// - Düğüm kimliği ayrıştırılmaz: hedef `ddl_target_of` ile alınır (`create`/`object`/`refreshNodeId`).
// - Kilitli sunucuda değiştiren açma reddedilir (menü zaten kapalıdır; savunma). Önizleme-only açma burada sunulmaz.
// - Tek seferde bir açma (`opening`); `openCreate`/`openAlter`, `openDesigner` yeni sekme ürettikten hemen sonra
//   EŞZAMANLI çağrılır (sekme çizilmeden denetleyici girdisi hazır olsun).
// - Sunucu kapsamlı türler (Database/Role; Plan 14): `databaseOid` her zaman `null` gönderilir (hedef zaten null
//   döner; savunma), sekme başlığında/`where`'de veritabanı adı yoktur. Veritabanı kapsamlı türde `null` → açılmaz.
// - Yetkiler (Plan 22): `privileges` nesnenin ACL sekmesini (`openAcl`, anahtar `acl:…`), `defaultPrivileges` yalnız
//   `db`/`sch` düğümünde varsayılan yetkiler sekmesini (`openDefaultAcl`, anahtar `defacl:…`) açar. Varsayılan
//   yetkiler her zaman veritabanı kapsamlıdır (`db` düğümünde `object.oid`). Kilit/bağlantı akışı aynıdır.
import { createRoot, createEffect, on } from "solid-js";
import { t } from "../../app/strings";
import { tdes } from "../../app/strings-designer";
import type { ToastKind } from "../../app/toast-model";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { Api } from "../../ipc/commands";
import { errorText, toPayload } from "../../ipc/errors";
import type { DesignerOpenMode } from "../explorer/actions";
import type { ExplorerController } from "../explorer/controller";
import { ancestorOfKind, type TreeState } from "../explorer/tree-model";
import type { QueryController } from "../query/query-controller";
import type { TabsStore } from "../query/tabs-store";
import { isProductionLabel } from "../servers/server-form";
import { createDesignerController, type DesignerController, type DesignerControllerDeps } from "./designer-controller";
import { aclTabKey, defaultAclTabKey, designerTabKey, focusFieldOf, isServerScoped, kindLabel } from "./designer-model";
import type { DropInput } from "./drop-controller";
import { extraRefreshIds, titleAfterApply, type OpenContext } from "./refresh-model";

export type { DropInput };

export interface DesignerOpenDeps {
  designer: Pick<DesignerController, "openCreate" | "openAlter" | "openAcl" | "openDefaultAcl" | "requestFocus" | "findAlter" | "ids">;
  tabs: Pick<TabsStore, "state" | "openDesigner" | "activate" | "rename">;
  api: Pick<Api, "ddlTargetOf">;
  tree: () => TreeState;
  /** Güncel sunucu kayıtları (kilit, ad). */
  refreshServers: () => Promise<ServerConfig[]>;
  /** Sunucuyu katalog tarafında bağlar (gerekirse parola sorar); bağlıysa hemen `true`. */
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  /** Silme diyaloğu (sekme açmaz). */
  openDrop(input: DropInput): void;
}

export function createDesignerOpener(deps: DesignerOpenDeps) {
  const { designer, tabs, notify } = deps;
  let opening = false;
  /** "<sunucu>|<oid>" → veritabanı adı (betiği Query Tool'da açarken sekme başlığı için). */
  const dbNames = new Map<string, string>();
  /** Sekme → açılış bağlamı (uygulama sonrası ek yenilemeler); kapanan sekmelerin girdileri her açılışta budanır. */
  const contexts = new Map<string, OpenContext>();
  const remember = (tabId: string, ctx: OpenContext) => {
    const live = new Set(designer.ids());
    for (const k of contexts.keys()) if (!live.has(k)) contexts.delete(k);
    contexts.set(tabId, ctx);
  };

  const open = async (nodeId: string, mode: DesignerOpenMode): Promise<void> => {
    if (opening) return;
    const node = deps.tree().nodes.get(nodeId);
    if (!node) return;
    opening = true;
    try {
      const [target, list] = await Promise.all([deps.api.ddlTargetOf(nodeId), deps.refreshServers()]);
      const { refreshNodeId } = target;
      const create = mode === "create" ? target.create : null;
      const object = mode === "create" ? null : target.object;
      const kind = create?.kind ?? object?.kind;
      // Kolon/kısıt odağı olan hedef (tablo diyaloğu) hiçbir zaman DROP TABLE/yeniden adlandırma diyaloğu açmaz.
      if (target.focus && (mode === "drop" || mode === "rename")) {
        notify(tdes.open.rowInTable, "info");
        return;
      }
      // Plan 22: varsayılan yetkiler yalnız veritabanı ve şema düğümünde; kolon/kısıt odak hedefi (tabloya çözülür)
      // yetki sekmesi açmaz (menü göstermez; savunma).
      const defAcl = mode === "defaultPrivileges";
      if (
        kind === undefined || (target.databaseOid === null && !isServerScoped(kind))
        || (defAcl && kind !== "database" && kind !== "schema") || ((defAcl || mode === "privileges") && target.focus)
      ) {
        notify(tdes.open.none, "info");
        return;
      }
      const config = list.find((x) => x.id === target.serverId);
      if (!config) {
        notify(t.query.ctx.serverGone, "error");
        return;
      }
      if (config.readOnlyLock) {
        notify(tdes.open.locked, "error");
        return;
      }
      if (!(await deps.connect(config.id))) return;
      const serverId = config.id;
      const serverScoped = isServerScoped(kind);
      const databaseOid = serverScoped ? null : target.databaseOid;
      const database = serverScoped ? null : (ancestorOfKind(deps.tree(), nodeId, ["database"])?.label ?? null);
      if (database !== null && databaseOid !== null) dbNames.set(`${serverId}|${databaseOid}`, database);
      const where = database === null ? config.name : `${config.name}/${database}`;
      if (object && mode === "drop") {
        deps.openDrop({ serverId, databaseOid, object, refreshNodeId, name: node.label });
        return;
      }
      if (create) {
        const key = designerTabKey(tabs.state.designers, { serverId, databaseOid, create });
        const title = tdes.tab.create(kindLabel(create.kind), config.name, database);
        const r = tabs.openDesigner({ serverId, databaseOid, title, key, where });
        if (r.created) void designer.openCreate({ tabId: r.id, serverId, databaseOid, create, refreshNodeId });
        remember(r.id, {
          databaseNodeId: ancestorOfKind(deps.tree(), nodeId, ["database"])?.id ?? null, tableNodeId: null, collectionNodeId: refreshNodeId,
        });
        return;
      }
      if (!object) return;
      if (mode === "privileges" || defAcl) {
        const ctx: OpenContext = { databaseNodeId: ancestorOfKind(deps.tree(), nodeId, ["database"])?.id ?? null, tableNodeId: null };
        let key: string;
        let title: string;
        let tabDb: number | null = databaseOid;
        let tabWhere = where;
        if (defAcl) {
          // Varsayılan yetkiler veritabanı kapsamlıdır: `db` düğümünde veritabanı nesnenin kendisi, `sch`'de hedefin veritabanı.
          tabDb = object.kind === "database" ? object.oid : target.databaseOid;
          if (tabDb === null) return;
          const dbLabel = ancestorOfKind(deps.tree(), nodeId, ["database"])?.label ?? null;
          if (dbLabel !== null) {
            dbNames.set(`${serverId}|${tabDb}`, dbLabel);
            tabWhere = `${config.name}/${dbLabel}`;
          }
          key = defaultAclTabKey(serverId, tabDb, object.kind === "schema" ? object.oid : null);
          title = tdes.tab.defaultPrivileges(node.label);
        } else {
          key = aclTabKey(object.kind, serverId, databaseOid, object.oid);
          title = tdes.tab.privileges(node.label);
        }
        const r = tabs.openDesigner({ serverId, databaseOid: tabDb, title, key, where: tabWhere });
        const actx = { serverId, databaseOid: tabDb, refreshNodeId };
        if (r.created) {
          if (defAcl) void designer.openDefaultAcl(r.id, actx, { role: null, schemaOid: object.kind === "schema" ? object.oid : null });
          else void designer.openAcl(r.id, actx, object);
        }
        remember(r.id, ctx);
        return;
      }
      const focus = (id: string) => {
        if (mode === "rename") designer.requestFocus(id, "name");
        else if (target.focus) designer.requestFocus(id, focusFieldOf(target.focus));
      };
      const tableNode = object.kind === "table" ? (node.kind === "table" ? node : ancestorOfKind(deps.tree(), nodeId, ["table"])) : null;
      const ctx: OpenContext = { databaseNodeId: ancestorOfKind(deps.tree(), nodeId, ["database"])?.id ?? null, tableNodeId: tableNode?.id ?? null };
      // Create'ten dönüşen ya da kimliği değişen sekme de eşlenir (sekme anahtarı değişmez).
      const existing = designer.findAlter(serverId, databaseOid, object);
      if (existing !== null) {
        tabs.activate(existing);
        remember(existing, ctx);
        focus(existing);
        return;
      }
      const key = designerTabKey(tabs.state.designers, { serverId, databaseOid, object });
      const title = tdes.tab.edit(kindLabel(object.kind), (tableNode ?? node).label);
      // Not: oid'i değişen alter'dan (indeks yeniden kurma) sonra sekme anahtarı eski oid'de kalır; PostgreSQL aynı oid'i
      // oturum içinde başka nesneye verirse `openDesigner` eski sekmeyi etkinleştirir. Kuramsal; bilerek düzeltilmedi.
      const r = tabs.openDesigner({ serverId, databaseOid, title, key, where });
      if (r.created) void designer.openAlter({ tabId: r.id, serverId, databaseOid, object, refreshNodeId });
      remember(r.id, ctx);
      focus(r.id);
    } catch (err) {
      notify(errorText(toPayload(err)), "error");
    } finally {
      opening = false;
    }
  };

  return {
    open,
    context: (tabId: string): OpenContext | null => contexts.get(tabId) ?? null,
    /** Veritabanı kapsamlı hedefin veritabanı adı; sunucu kapsamlıda (`null`) her zaman null (bakım veritabanı). */
    databaseName: (serverId: string, databaseOid: number | null) =>
      (databaseOid === null ? null : (dbNames.get(`${serverId}|${databaseOid}`) ?? null)),
  };
}

export interface DesignerDeps extends Omit<DesignerOpenDeps, "designer" | "openDrop"> {
  api: DesignerControllerDeps["api"] & DesignerOpenDeps["api"];
  /**
   * `servers`: sunucu listesi (tepkisel; kayıt değişince açık sekmelerin ad/kilit/üretim bilgisi yenilenir);
   * `openScript`: "Query Tool'da aç" (ApplyBar) — betik yeni sorgu sekmesinde, çalıştırılmadan.
   */
  query: Pick<QueryController, "servers" | "openScript">;
  /** Uygulama sonrası düğüm yenilemesi (`refreshNodeId`; düğüm artık yoksa denetleyicinin hata yolu). */
  explorer: Pick<ExplorerController, "refresh">;
  /** Silme diyaloğu; verilmezse denetleyicinin tek örnek diyaloğu (`ctl.drop`, DesignerPanels çizer). */
  openDrop?: DesignerOpenDeps["openDrop"];
}

/** Uygulamanın tuttuğu designer nesnesi: denetleyici + açma akışı + betik açma. */
export function createDesigner(deps: DesignerDeps) {
  // Sekme başlıkları/panelleri hemen ardından gerekecek: açılış isteğiyle paralel insin.
  void import("./DesignerTabs").catch(() => {});
  const ctl = createDesignerController({
    api: deps.api,
    notify: deps.notify,
    serverInfo: (id) => {
      const c = deps.query.servers().find((s) => s.id === id);
      return c ? { name: c.name, locked: c.readOnlyLock, production: isProductionLabel(c.label), username: c.username } : null;
    },
    refreshNode: (id) => deps.explorer.refresh(id),
    onApplied: (info) => {
      const title = titleAfterApply(info);
      if (title) deps.tabs.rename(info.tabId, title);
      for (const id of extraRefreshIds(deps.tree(), opener.context(info.tabId), info, null)) void Promise.resolve(deps.explorer.refresh(id)).catch(() => {});
    },
  });
  // Sunucu kaydı değişince (ad/kilit/etiket) açık sekmeler güncellenir; kök uygulama ömrü boyunca yaşar.
  createRoot(() => createEffect(on(deps.query.servers, (list) => list.forEach((s) => ctl.serverChanged(s.id)), { defer: true })));
  const opener: ReturnType<typeof createDesignerOpener> = createDesignerOpener({
    ...deps,
    designer: ctl,
    openDrop: deps.openDrop ?? ctl.drop.open,
  });
  return {
    ctl,
    open: opener.open,
    openScript: (input: { serverId: string; databaseOid: number | null; text: string }) =>
      void deps.query.openScript({ ...input, database: opener.databaseName(input.serverId, input.databaseOid) }),
  };
}

export type Designer = ReturnType<typeof createDesigner>;
