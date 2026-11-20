// Hata kartının metinleri (tembel parça: yalnız ErrorBadgeCard içe aktarır). Ana paketteki strings.ts'ten taşındı (Plan 37 telafisi).
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { isGone } from "./error-badge-model";

export const terr = { gone: "Nesne artık mevcut değil.", refreshParent: "Üst düğümü yenile", retry: "Tekrar dene" } as const;

/** Kartın başlık satırı. */
export const cardTitle = (err: ErrorPayload): string => (isGone(err) ? terr.gone : err.message);

/** Kartın eylem düğmesi etiketi. */
export const cardActionLabel = (err: ErrorPayload): string => (isGone(err) ? terr.refreshParent : terr.retry);
