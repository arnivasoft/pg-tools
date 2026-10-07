// Veri düzenleyicinin uygulamaya giriş noktası (tembel parça). Ana paket bu modülü yalnızca ilk "Verileri
// Görüntüle/Düzenle"de dinamik içe aktarır.
//
// Paket bütçesi (Plan 17 R1): bu modül BİLEREK bağımlılıksızdır (yalnızca ana paket modülleri ve türler). Solid ya da
// strings-data gibi başka bir parçayı statik içe aktarsaydı ana paketteki `import("./data-open")` çağrısı
// `__vite__mapDeps` girdisi kazanırdı (+52 B, 22 girdi; ölçüldü). Denetleyici ve açıcı bu yüzden sekme panelleriyle
// aynı tembel parçada (`DataTabs`, zaten ana listede) iner; buradaki `ctl` yüzü yüklenene dek güvenli varsayılanlar
// döner (veri sekmesi yalnızca açıcıdan, yani yüklemeden SONRA doğar).
import type { ToastKind } from "../../app/toast-model";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import type { TreeState } from "../explorer/tree-model";
import type { QueryController } from "../query/query-controller";
import type { TabsStore } from "../query/tabs-store";
import type { DataTabCtl } from "./data-controller";

/** QueryTabs'ın, App'in ve sekme panellerinin gördüğü denetleyici yüzü. */
export type DataController = DataTabCtl;

export interface DataDeps {
  api: Api;
  tree: () => TreeState;
  tabs: TabsStore;
  refreshServers: () => Promise<ServerConfig[]>;
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  /** Tepkisel sunucu listesi (kayıt değişince kilit/üretim/ad tazelenir); yoksa açıcının tazelemeleri kullanılır. */
  query?: Pick<QueryController, "servers">;
}

interface Inner {
  ctl: DataTabCtl;
  open: (nodeId: string) => Promise<void>;
}

export function createData(deps: DataDeps) {
  let inner: Inner | null = null;
  let loading: Promise<Inner> | null = null;
  const load = () =>
    (loading ??= import("./DataTabs").then(
      (m) => (inner = m.startData(deps)),
      (err: unknown) => {
        loading = null;
        throw err;
      },
    ));
  // Sekme başlıkları/panelleri ve denetleyici hemen ardından gerekecek: açılış isteğiyle paralel insin.
  void load().catch(() => {});
  const i = () => inner?.ctl;
  const ctl: DataController = {
    model: (id) => i()?.model(id) ?? null,
    ids: () => i()?.ids() ?? [],
    isDirty: (id) => i()?.isDirty(id) ?? false,
    openTab: (input) => i()?.openTab(input) ?? input.tabId,
    reload: (id, opts) => i()?.reload(id, opts) ?? Promise.resolve(false),
    reloadNow: (id) => i()?.reloadNow(id) ?? false,
    answerReload: (id, discard) => i()?.answerReload(id, discard) ?? Promise.resolve(false),
    reconnect: (id) => i()?.reconnect(id) ?? Promise.resolve(false),
    loadMore: (id) => i()?.loadMore(id) ?? Promise.resolve(),
    setSelect: (id, next, opts) => i()?.setSelect(id, next, opts) ?? Promise.resolve(false),
    edit: (id, fn) => i()?.edit(id, fn),
    setConfirm: (id, checked) => i()?.setConfirm(id, checked),
    apply: (id) => i()?.apply(id) ?? Promise.resolve(),
    cancelApply: (id) => i()?.cancelApply(id),
    close: (id) => i()?.close(id),
    serverSessionsClosed: (serverId, gone) => i()?.serverSessionsClosed(serverId, gone),
    serverChanged: (serverId) => i()?.serverChanged(serverId),
  };
  const open = (nodeId: string): Promise<void> =>
    load().then(
      (x) => x.open(nodeId),
      (err: unknown) => deps.notify(toPayload(err).message, "error"),
    );
  return { ctl, open };
}

export type Data = ReturnType<typeof createData>;
