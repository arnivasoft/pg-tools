// View / Materialized View formunun saf modeli (tembel parça): tür daraltma, alan güncelleyicileri ve form kuralları
// (yeniden oluştur görünürlüğü, kaybolacaklar, REFRESH seçenekleri). SQL üretmez; sorgu olduğu gibi gider.
import { tview } from "../../app/strings-view";
import type { CheckOption } from "../../ipc/bindings/CheckOption";
import type { MatViewDef } from "../../ipc/bindings/MatViewDef";
import type { MatViewRefresh } from "../../ipc/bindings/MatViewRefresh";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ViewDef } from "../../ipc/bindings/ViewDef";

export type ViewDraft = ViewDef & { kind: "view" };
export type MatViewDraft = MatViewDef & { kind: "materialized_view" };

export function asView(d: ObjectDef | null): ViewDraft | null {
  return d && d.kind === "view" ? d : null;
}
export function asMatView(d: ObjectDef | null): MatViewDraft | null {
  return d && d.kind === "materialized_view" ? d : null;
}

/** Görünüm taslağına alan güncellemesi uygular (tür uyuşmazsa tanım değişmez). */
export function viewEditPatch(d: ObjectDef, patch: Partial<ViewDef>): ObjectDef {
  return d.kind === "view" ? { ...d, ...patch } : d;
}
export function matviewEditPatch(d: ObjectDef, patch: Partial<MatViewDef>): ObjectDef {
  return d.kind === "materialized_view" ? { ...d, ...patch } : d;
}

/** Alter: sorgu yüklenenden farklı mı (metin eşitliği; boşluk dahil — arka uç da metin karşılaştırır). */
export function queryChanged(original: ViewDef | MatViewDef | null, d: ViewDef | MatViewDef): boolean {
  return original !== null && original.query !== d.query;
}

/** "Yeniden oluştur" kutusu görünür mü (yalnız alter + view + sorgu değişmiş). */
export function recreateVisible(mode: "create" | "alter", original: ViewDef | null, d: ViewDef): boolean {
  return mode === "alter" && original !== null && queryChanged(original, d);
}

/** Sorgu düzenlemesi; view'da sorgu özgüne dönerse `recreate` false'a çekilir (kutu gizlenir, bayrak sızmaz). */
export function withQuery<T extends ViewDef | MatViewDef>(d: T, original: T | null, query: string): T {
  const next = { ...d, query };
  if ("recreate" in next && original !== null && original.query === query) return { ...next, recreate: false };
  return next;
}

/** Yeniden kurmada kaybolacaklar (yüklenen `unsupported` + her zaman "GRANT yetkileri"); create'te []. */
export function lossList(original: ViewDef | MatViewDef | null): string[] {
  return original === null ? [] : [...original.unsupported, tview.grants];
}

/** MV yenileme seçenekleri (Plan 14 Task 3 tablosu): sunulan `refresh` değerleri. */
export function refreshChoices(original: MatViewDef | null, d: MatViewDef): Array<MatViewRefresh | null> {
  if (original === null || queryChanged(original, d)) return [null];
  if (!original.withData && d.withData) return [null, "normal"];
  if (original.withData && d.withData) return [null, "normal", "concurrently"];
  return [null];
}

/** withData değişimi: refresh yeni seçeneklerde yoksa null'a çekilir. */
export function withDataPatch(original: MatViewDef | null, d: MatViewDef, withData: boolean): MatViewDef {
  const next = { ...d, withData };
  return refreshChoices(original, next).includes(next.refresh) ? next : { ...next, refresh: null };
}

/** MV tanım değişimi (sorgu) → "yeniden kurulur" uyarısı ve refresh kilitli (null). */
export function matviewRebuild(original: MatViewDef | null, d: MatViewDef): boolean {
  return queryChanged(original, d);
}

export function checkOptionFromSelect(v: string): CheckOption | null {
  return v === "local" || v === "cascaded" ? v : null;
}
