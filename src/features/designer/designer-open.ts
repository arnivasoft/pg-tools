// Nesne diyaloğunun (designer) uygulamaya giriş noktası (tembel parça): denetleyiciyi kurar ve gezgin düğümünden açma
// akışını taşır. Ana paket bu modülü yalnızca ilk "Oluştur/Düzenle/Yeniden adlandır/Sil"de dinamik içe aktarır.
//
// Kurallar:
// - Düğüm kimliği ayrıştırılmaz: hedef `ddl_target_of` ile alınır (`create`/`object`/`refreshNodeId`).
// - Kilitli sunucuda değiştiren açma reddedilir (menü zaten kapalıdır; savunma). Önizleme-only açma burada sunulmaz.
// - Tek seferde bir açma (`opening`); `openCreate`/`openAlter`, `openDesigner` yeni sekme ürettikten hemen sonra
//   EŞZAMANLI çağrılır (sekme çizilmeden denetleyici girdisi hazır olsun).
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
import { designerTabKey, kindLabel } from "./designer-model";
import type { DropInput } from "./drop-controller";

export type { DropInput };

export interface DesignerOpenDeps {
  designer: DesignerController;
  tabs: Pick<TabsStore, "state" | "openDesigner">;
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

  const open = async (nodeId: string, mode: DesignerOpenMode): Promise<void> => {
    if (opening) return;
    const node = deps.tree().nodes.get(nodeId);
    if (!node) return;
    opening = true;
    try {
      const [target, list] = await Promise.all([deps.api.ddlTargetOf(nodeId), deps.refreshServers()]);
      const { databaseOid, refreshNodeId } = target;
      const create = mode === "create" ? target.create : null;
      const object = mode === "create" ? null : target.object;
      // Kolon/kısıt odağı olan hedef (tablo diyaloğu) hiçbir zaman DROP TABLE/yeniden adlandırma diyaloğu açmaz.
      if (target.focus && (mode === "drop" || mode === "rename")) {
        notify(tdes.open.none, "info");
        return;
      }
      if ((!create && !object) || databaseOid === null) {
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
      const database = ancestorOfKind(deps.tree(), nodeId, ["database"])?.label ?? null;
      if (database !== null) dbNames.set(`${serverId}|${databaseOid}`, database);
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
        return;
      }
      if (!object) return;
      const key = designerTabKey(tabs.state.designers, { serverId, databaseOid, object });
      const title = tdes.tab.edit(kindLabel(object.kind), node.label);
      const r = tabs.openDesigner({ serverId, databaseOid, title, key, where });
      if (r.created) void designer.openAlter({ tabId: r.id, serverId, databaseOid, object, refreshNodeId });
      if (mode === "rename") designer.requestFocus(r.id, "name");
    } catch (err) {
      notify(errorText(toPayload(err)), "error");
    } finally {
      opening = false;
    }
  };

  return { open, databaseName: (serverId: string, databaseOid: number) => dbNames.get(`${serverId}|${databaseOid}`) ?? null };
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
      return c ? { name: c.name, locked: c.readOnlyLock, production: isProductionLabel(c.label) } : null;
    },
    refreshNode: (id) => deps.explorer.refresh(id),
  });
  // Sunucu kaydı değişince (ad/kilit/etiket) açık sekmeler güncellenir; kök uygulama ömrü boyunca yaşar.
  createRoot(() => createEffect(on(deps.query.servers, (list) => list.forEach((s) => ctl.serverChanged(s.id)), { defer: true })));
  const opener = createDesignerOpener({
    ...deps,
    designer: ctl,
    openDrop: deps.openDrop ?? ctl.drop.open,
  });
  return {
    ctl,
    open: opener.open,
    openScript: (input: { serverId: string; databaseOid: number; text: string }) =>
      void deps.query.openScript({ ...input, database: opener.databaseName(input.serverId, input.databaseOid) }),
  };
}

export type Designer = ReturnType<typeof createDesigner>;
