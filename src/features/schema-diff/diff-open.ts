// Şema karşılaştırmanın uygulamaya giriş noktası (tembel parça). Ana paket bu modülü yalnızca ilk "Şema karşılaştır…"da
// dinamik içe aktarır.
//
// Paket bütçesi: bu modül BİLEREK bağımlılıksızdır (yalnızca ana paket modülleri ve türler; çalışma zamanında yalnızca
// `toPayload`). Solid ya da strings-diff gibi başka bir parçayı statik içe aktarsaydı ana paketteki `import("./diff-open")`
// çağrısı `__vite__mapDeps` girdisi kazanırdı (bkz. data-open.ts). Denetleyici ve açıcı bu yüzden sekme panelleriyle aynı
// tembel parçada (`DiffTabs`) iner; buradaki `ctl` yüzü yüklenene dek güvenli varsayılanlar döner.
import type { ToastKind } from "../../app/toast-model";
import type { Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import type { TreeState } from "../explorer/tree-model";
import type { QueryController } from "../query/query-controller";
import type { TabsStore } from "../query/tabs-store";
import type { DiffTabCtl } from "./diff-controller";

export type DiffController = DiffTabCtl;

export interface DiffDeps {
  api: Api;
  tree: () => TreeState;
  tabs: TabsStore;
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  query: Pick<QueryController, "servers" | "openScript">;
}

interface Inner {
  ctl: DiffTabCtl;
  open: (nodeId: string) => Promise<void>;
}

export function createSchemaDiff(deps: DiffDeps) {
  let inner: Inner | null = null;
  let loading: Promise<Inner> | null = null;
  const load = () =>
    (loading ??= import("./DiffTabs").then(
      (m) => (inner = m.startDiff(deps)),
      (err: unknown) => {
        loading = null;
        throw err;
      },
    ));
  // Sekme başlıkları/panelleri ve denetleyici hemen ardından gerekecek: açılış isteğiyle paralel insin.
  void load().catch(() => {});
  const i = () => inner?.ctl;
  const done = Promise.resolve();
  const ctl: DiffController = {
    model: (id) => i()?.model(id) ?? null,
    ids: () => i()?.ids() ?? [],
    open: (nodeId) => i()?.open(nodeId) ?? done,
    close: (id) => i()?.close(id),
    setServer: (id, side, serverId) => i()?.setServer(id, side, serverId) ?? done,
    setDatabase: (id, side, oid) => i()?.setDatabase(id, side, oid) ?? done,
    setSchema: (id, side, schema) => i()?.setSchema(id, side, schema),
    swap: (id) => i()?.swap(id),
    setOption: (id, key, value) => i()?.setOption(id, key, value),
    compare: (id) => i()?.compare(id) ?? done,
    cancel: (id) => i()?.cancel(id),
    setFilter: (id, c, on) => i()?.setFilter(id, c, on),
    setSearch: (id, q) => i()?.setSearch(id, q),
    toggle: (id, key) => i()?.toggle(id, key),
    selectAll: (id, on) => i()?.selectAll(id, on),
    selectVisible: (id, on) => i()?.selectVisible(id, on),
    focus: (id, key) => i()?.focus(id, key),
    moveFocus: (id, delta) => i()?.moveFocus(id, delta),
    buildScript: (id) => i()?.buildScript(id) ?? done,
    closeScript: (id) => i()?.closeScript(id),
    openInQueryTool: (id) => i()?.openInQueryTool(id) ?? done,
    copyScript: (id) => i()?.copyScript(id) ?? done,
    saveScript: (id) => i()?.saveScript(id) ?? done,
  };
  const open = (nodeId: string): Promise<void> =>
    load().then(
      (x) => x.open(nodeId),
      (err: unknown) => deps.notify(toPayload(err).message, "error"),
    );
  return { ctl, open };
}

export type SchemaDiff = ReturnType<typeof createSchemaDiff>;
