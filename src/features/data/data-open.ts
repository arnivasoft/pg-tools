// Veri düzenleyicinin uygulamaya giriş noktası (tembel parça). Ana paket bu modülü yalnızca ilk "Verileri
// Görüntüle/Düzenle"de dinamik içe aktarır. TASLAK (Plan 17 Task 1): gövdeler sonraki görevlerde doldurulur.
import type { ToastKind } from "../../app/toast-model";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { Api } from "../../ipc/commands";
import type { TreeState } from "../explorer/tree-model";
import type { TabsStore } from "../query/tabs-store";

/** QueryTabs'ın ve App'in gördüğü denetleyici yüzü. */
export interface DataController {
  /** Kaydedilmemiş değişiklik var mı (kapatma/yeniden yükleme onayı)? */
  isDirty(id: string): boolean;
  /** Sekme girdisini bırakır (oturum kapanır, uçuştaki uygulama iptal edilir). */
  close(id: string): void;
  /** F5: kaynağı ve satırları yeniden yükler (kirli ise onay ister). */
  reloadNow(id: string): void;
  /** Sunucunun oturumları kapandı (bağlantı kesme/silme/kayıt değişimi). */
  serverSessionsClosed(serverId: string, gone?: boolean): void;
}

export interface DataDeps {
  api: Api;
  tree: () => TreeState;
  tabs: TabsStore;
  refreshServers: () => Promise<ServerConfig[]>;
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
}

export function createData(_deps: DataDeps) {
  // Sekme başlıkları/panelleri hemen ardından gerekecek: açılış isteğiyle paralel insin.
  void import("./DataTabs").catch(() => {});
  const ctl: DataController = {
    isDirty: () => false,
    close: () => {},
    reloadNow: () => {},
    serverSessionsClosed: () => {},
  };
  return { ctl, open: (_nodeId: string): void => {} };
}

export type Data = ReturnType<typeof createData>;
