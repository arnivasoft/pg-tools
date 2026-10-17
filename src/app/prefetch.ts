// Tembel parçaların boşta ısıtılması. Yükleyiciler DIŞARIDAN verilir (burada statik/dinamik içe aktarma yok): aksi halde
// paketleyici bu modülü tembel parçalara bağlayıp açılış kümesine geri taşırdı. Yalnız ısıtmadır: hatalar sessizce yutulur
// (bildirim yok); asıl kullanımda ilgili `createLoader` yeniden dener ve kullanıcıya bildirir.
type Schedule = (cb: () => void) => void;

const defaultSchedule: Schedule = (cb) => {
  if (typeof requestIdleCallback === "function") requestIdleCallback(() => cb());
  else setTimeout(cb, 1500);
};

export function prefetchIdle(loaders: (() => Promise<unknown>)[], schedule: Schedule = defaultSchedule): void {
  schedule(() => {
    for (const load of loaders) {
      // Ret ya da eşzamanlı fırlatma diğer yükleyicileri engellemez.
      try {
        load().catch(() => {});
      } catch {
        // eşzamanlı fırlatma da yutulur
      }
    }
  });
}
