// Veri düzenleyicinin açma akışı ve kurulumu (tembel parça; DataTabs parçasıyla birlikte iner — bkz. data-open.ts).
// designer-open.ts kalıbı (tek uçuşlu `opening`, `ddl_target_of` + sunucu tazeleme, bağlan, sekme anahtarı); designer/*
// içe aktarılmaz (Karar 2).
//
// Kurallar:
// - Düğüm kimliği ayrıştırılmaz: hedef `ddl_target_of` ile alınır. Yalnızca nesne hedefi (`focus` yok), veritabanı
//   kapsamlı (`databaseOid` dolu) ve tür ∈ {table, view, materialized_view} açılır; aksi halde `tdata.open.none`.
// - Kilitli sunucuda da açılır (salt okunur şeridi ve kapalı kapı denetleyicidedir).
// - Aynı nesne tek sekme (`data:<sunucu>:<veritabanı oid>:<nesne oid>`); mevcutsa etkinleşir.
import { createEffect, createRoot, createSignal, on, type Accessor } from "solid-js";
import { t } from "../../app/strings";
import { tdata } from "../../app/strings-data";
import type { ToastKind } from "../../app/toast-model";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import { errorText, toPayload } from "../../ipc/errors";
import { ancestorOfKind, type TreeState } from "../explorer/tree-model";
import type { TabsStore } from "../query/tabs-store";
import { isProductionLabel } from "../servers/server-form";
import { createDataApi, type DataApi } from "./data-commands";
import { createDataController, type DataTabCtl } from "./data-controller";
import type { DataDeps } from "./data-open";

const OPEN_KINDS: readonly ObjectKind[] = ["table", "view", "materialized_view"];

export const dataTabKey = (serverId: string, databaseOid: number, oid: number) => `data:${serverId}:${databaseOid}:${oid}`;

export interface DataOpenDeps {
  ctl: Pick<DataTabCtl, "openTab">;
  tabs: Pick<TabsStore, "openData">;
  api: Pick<DataApi, "ddlTargetOf">;
  tree: () => TreeState;
  refreshServers: () => Promise<ServerConfig[]>;
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
}

export function createDataOpener(deps: DataOpenDeps) {
  let opening = false;
  const open = async (nodeId: string): Promise<void> => {
    if (opening) return;
    const node = deps.tree().nodes.get(nodeId);
    if (!node) return;
    opening = true;
    try {
      const [target, list] = await Promise.all([deps.api.ddlTargetOf(nodeId), deps.refreshServers()]);
      const { object, databaseOid } = target;
      if (!object || target.focus !== null || databaseOid === null || !OPEN_KINDS.includes(object.kind)) {
        deps.notify(tdata.open.none, "info");
        return;
      }
      const config = list.find((x) => x.id === target.serverId);
      if (!config) {
        deps.notify(t.query.ctx.serverGone, "error");
        return;
      }
      if (!(await deps.connect(config.id))) return;
      const tree = deps.tree();
      const database = ancestorOfKind(tree, nodeId, ["database"])?.label ?? "";
      const schema = ancestorOfKind(tree, nodeId, ["schema"])?.label ?? null;
      const where = schema === null ? `${config.name}/${database}` : `${config.name}/${database}.${schema}`;
      const r = deps.tabs.openData({
        serverId: config.id, databaseOid, object: { ...object }, title: tdata.tab.title(node.label),
        key: dataTabKey(config.id, databaseOid, object.oid), where,
      });
      if (r.created) deps.ctl.openTab({ tabId: r.id, serverId: config.id, databaseOid, object: { ...object } });
    } catch (err) {
      // Yalnızca hata iletisi (düğüm/sunucu); hücre değeri içermez.
      deps.notify(errorText(toPayload(err)), "error");
    } finally {
      opening = false;
    }
  };
  return { open };
}

/** Denetleyici + açıcı; sunucu kaydı değişince açık sekmelerin ad/kilit/üretim bilgisi tazelenir. */
export function startData(deps: DataDeps) {
  // Sunucu listesi: uygulama sinyali verilmişse o (tepkisel), yoksa açıcının her tazelemesiyle güncellenen yerel kopya.
  const [cache, setCache] = createSignal<ServerConfig[]>([]);
  const servers: Accessor<ServerConfig[]> = deps.query?.servers ?? cache;
  const refreshServers = async () => {
    const list = await deps.refreshServers();
    setCache(list);
    return list;
  };
  const api = createDataApi(deps.api, refreshServers, deps.connect);
  const ctl = createDataController({
    api,
    serverInfo: (id) => {
      const c = servers().find((s) => s.id === id);
      return c ? { name: c.name, locked: c.readOnlyLock, production: isProductionLabel(c.label) } : null;
    },
  });
  // Kök uygulama ömrü boyunca yaşar.
  createRoot(() => createEffect(on(servers, (list) => list.forEach((s) => ctl.serverChanged(s.id)), { defer: true })));
  const opener = createDataOpener({ ctl, tabs: deps.tabs, api, tree: deps.tree, refreshServers, connect: deps.connect, notify: deps.notify });
  return { ctl, open: opener.open };
}
