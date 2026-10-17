// Hata rozetinin saf mantığı (ana paket): tam bileşen (tembel) ile fallback aynı etiketi/başlığı kullanır.
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";

/** Rozetin `aria-label`ı (tek kaynak: fallback ve HoverCard'lı tetikleyici aynı değeri üretir). */
export const badgeLabel = (): string => t.error.badge;

/** Kaybolmuş nesne: yeniden denemek yerine üst düğüm yenilenir. */
export const isGone = (err: ErrorPayload): boolean => err.code === "not_found";

/** Kartın başlık satırı. */
export const cardTitle = (err: ErrorPayload): string => (isGone(err) ? t.error.gone : err.message);

/** Kartın eylem düğmesi etiketi. */
export const cardActionLabel = (err: ErrorPayload): string => (isGone(err) ? t.error.refreshParent : t.error.retry);

/** Rozet ortak sınıfları (fallback ile tam tetikleyici birebir aynı görünür). */
export const BADGE_CLASS = "ml-auto shrink-0 rounded text-danger outline-none focus-visible:ring-1 focus-visible:ring-accent";

/**
 * Kart parçası yükleyicisi (`createLoader` ile aynı sözleşme): eşzamanlı istekler tek yüklemeyi paylaşır, ret başına `onFail`
 * yalnız bir kez çağrılır (rozet başına değil); ret önbelleğe alınmaz, sonraki istek yeniden dener. Başarıda `onLoaded(modül)`.
 */
export function createCardLoader<T>(load: () => Promise<T>, onLoaded: (m: T) => void, onFail: (err: unknown) => void): () => Promise<boolean> {
  let p: Promise<boolean> | null = null;
  return () =>
    (p ??= load().then(
      (m) => (onLoaded(m), true),
      (err) => ((p = null), onFail(err), false),
    ));
}
