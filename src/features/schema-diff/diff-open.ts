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
  const ctl: DiffController = {
    model: (id) => i()?.model(id) ?? null,
    ids: () => i()?.ids() ?? [],
    close: (id) => i()?.close(id),
  };
  const open = (nodeId: string): Promise<void> =>
    load().then(
      (x) => x.open(nodeId),
      (err: unknown) => deps.notify(toPayload(err).message, "error"),
    );
  return { ctl, open };
}

export type SchemaDiff = ReturnType<typeof createSchemaDiff>;
