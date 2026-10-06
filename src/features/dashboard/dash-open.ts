// Dashboard'un uygulamaya giriş noktası (tembel parça): denetleyiciyi kurar, gezgin düğümünden açma ve kayıp sekmeyi
// yeniden açma akışlarını taşır. Ana paket yalnızca bu modülü ilk "Dashboard'u aç"ta dinamik içe aktarır.
import { t } from "../../app/strings";
import { td } from "../../app/strings-dashboard";
import type { ToastKind } from "../../app/toast-model";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { Api } from "../../ipc/commands";
import { errorText, toPayload } from "../../ipc/errors";
import type { TreeState } from "../explorer/tree-model";
import type { TabsStore } from "../query/tabs-store";
import { createDashController, type DashController, type DashControllerDeps } from "./dash-controller";

export interface DashOpenDeps {
  dash: DashController;
  api: Pick<Api, "qtTargetOf">;
  tree: () => TreeState;
  tabs: Pick<TabsStore, "state" | "openDashboard">;
  /** Güncel sunucu kayıtları (sekme başlığı ve rozetler için). */
  refreshServers: () => Promise<ServerConfig[]>;
  /** Sunucuyu katalog tarafında bağlar (gerekirse parola sorar); bağlıysa hemen `true`. */
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
}

export function createDashOpener(deps: DashOpenDeps) {
  const { dash, tabs, notify } = deps;
  let opening = false;

  /** Gezgin düğümünden: aynı sunucu + veritabanı açıksa o sekme etkinleşir. Tek seferde bir açma. */
  const openFromNode = async (nodeId: string): Promise<void> => {
    if (opening) return;
    const node = deps.tree().nodes.get(nodeId);
    if (!node) return;
    opening = true;
    try {
      const [target, list] = await Promise.all([deps.api.qtTargetOf(nodeId), deps.refreshServers()]);
      const config = list.find((x) => x.id === target.serverId);
      if (!config) {
        notify(t.query.ctx.serverGone, "error");
        return;
      }
      if (!(await deps.connect(config.id))) return;
      const title = td.tabTitle(config.name, node.kind === "database" ? node.label : null);
      const r = tabs.openDashboard({ serverId: config.id, databaseOid: target.databaseOid, title });
      if (!r.created) {
        // Var olan sekme kayıpsa (bağlantı kesilmişti) sunucu az önce bağlandı: "Yeniden aç" ile aynı akış.
        if (dash.model(r.id)?.status === "lost") void dash.reopen(r.id);
        return;
      }
      // `open` modeli eşzamanlı kurar (ardından `setActive` onu görür); açılış hatası sekmede durum olarak görünür.
      void dash.open({ id: r.id, serverId: config.id, serverName: config.name, databaseOid: target.databaseOid });
      dash.setActive(tabs.state.activeId === r.id ? r.id : null);
    } catch (err) {
      notify(errorText(toPayload(err)), "error");
    } finally {
      opening = false;
    }
  };

  /** Kayıp sekme: kullanıcı bağlantıyı kestiyse önce sunucu yeniden bağlanır, sonra yeni `DashId`. */
  const reopen = async (tabId: string): Promise<void> => {
    const m = dash.model(tabId);
    if (m?.status !== "lost") return;
    if (await deps.connect(m.serverId)) await dash.reopen(tabId);
  };

  return { openFromNode, reopen };
}

/** Uygulamanın tuttuğu dashboard nesnesi: denetleyici + açma akışları. */
export function createDashboard(deps: Omit<DashOpenDeps, "dash"> & Pick<DashControllerDeps, "api">) {
  // Sekme başlıkları/panelleri hemen ardından gerekecek: açılış istekleriyle paralel insin (ilk sekme gecikmeden görünsün).
  void import("./DashTabs").catch(() => {});
  const ctl = createDashController({ api: deps.api, notify: deps.notify });
  return { ctl, ...createDashOpener({ ...deps, dash: ctl }) };
}

export type Dashboard = ReturnType<typeof createDashboard>;
