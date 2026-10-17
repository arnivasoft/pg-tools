// Silme onayı modeli (Solid bileşeni yok → birim test edilebilir): söz tabanlı istek + tembel görünüm yükleme.
import { createSignal } from "solid-js";

type Request = { name: string; resolve: (ok: boolean) => void };

/** `ensure`: görünüm parçasını yükler (başarıda true). Hata/iptal → istek `false` ile çözülür, diyalog açılmaz. */
export function createDeleteConfirmModel(ensure: () => Promise<boolean>) {
  const [request, setRequest] = createSignal<Request | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const ask = async (name: string) => {
    if (!(await ensure())) return false;
    return new Promise<boolean>((resolve) => {
      request()?.resolve(false);
      setLoaded(true);
      setRequest({ name, resolve });
    });
  };
  const finish = (ok: boolean) => {
    const cur = request();
    if (!cur) return;
    setRequest(null);
    cur.resolve(ok);
  };
  return { request, loaded, ask, finish };
}
