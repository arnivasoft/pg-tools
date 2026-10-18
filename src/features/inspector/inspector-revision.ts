// Inspector yenileme sayacı (ana paket). Tembel parçalar (designer, bakım) bunu içe aktarmaz: App `lazyDeps`'e
// `refreshInspector` olarak verir; başarılı uygulama sonrası seçili düğümün özellikleri ve DDL'i yeniden okunur.
import { createRoot, createSignal } from "solid-js";

const [revision, setRevision] = createRoot(() => createSignal(0));

export const inspectorRevision = revision;
export const refreshInspector = (): void => {
  setRevision((n) => n + 1);
};
