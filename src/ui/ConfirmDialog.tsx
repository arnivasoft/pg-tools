// Silme onayı (ana paket): yalnız model + tembel görünüm. `Dialog`/Kobalte burada statik içe aktarılmaz.
import { Suspense, Show, lazy } from "solid-js";
import { t } from "../app/strings";
import { toast } from "../app/toast";
import { createDeleteConfirmModel } from "./delete-confirm";
import { createLoader } from "./lazy-load";

/** Görünüm yükleyicisi (tek tanım): `lazy` ve ileride önyükleme aynı içe aktarmayı paylaşır. */
export const loadConfirmDialogView = () => import("./ConfirmDialogView");
const ConfirmDialogView = lazy(loadConfirmDialogView);

/** Söz tabanlı onay: `ask(name)` kullanıcı yanıtlayınca çözülür; `View` bir kez bağlanır. */
export function createDeleteConfirm() {
  const m = createDeleteConfirmModel(
    createLoader(loadConfirmDialogView, () => toast.show(t.app.chunkLoadFailed, "error")),
  );
  const View = () => (
    <Show when={m.loaded()}>
      <Suspense>
        <ConfirmDialogView
          open={m.request() !== null}
          title={t.menu.confirmTitle}
          message={t.menu.confirmMessage(m.request()?.name ?? "")}
          confirmLabel={t.menu.confirmLabel}
          danger
          onConfirm={() => m.finish(true)}
          onCancel={() => m.finish(false)}
        />
      </Suspense>
    </Show>
  );
  return { ask: m.ask, View };
}
