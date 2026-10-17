// Hata rozetinin saf mantığı (ana paket): tam bileşen (tembel) ile fallback aynı etiketi/başlığı kullanır.
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";

/** Rozetin `aria-label`ı (tek kaynak: fallback ve HoverCard'lı tetikleyici aynı değeri üretir). */
export const badgeLabel = (_err: ErrorPayload): string => t.error.badge;

/** Kaybolmuş nesne: yeniden denemek yerine üst düğüm yenilenir. */
export const isGone = (err: ErrorPayload): boolean => err.code === "not_found";

/** Kartın başlık satırı. */
export const cardTitle = (err: ErrorPayload): string => (isGone(err) ? t.error.gone : err.message);

/** Kartın eylem düğmesi etiketi. */
export const cardActionLabel = (err: ErrorPayload): string => (isGone(err) ? t.error.refreshParent : t.error.retry);

/** Rozet ortak sınıfları (fallback ile tam tetikleyici birebir aynı görünür). */
export const BADGE_CLASS = "ml-auto shrink-0 rounded text-danger outline-none focus-visible:ring-1 focus-visible:ring-accent";
