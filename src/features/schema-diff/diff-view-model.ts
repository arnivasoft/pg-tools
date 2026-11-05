// Şema karşılaştırma görünümlerinin saf yardımcıları (Plan 30): satır erişilebilirlik metni, risk tonu, satır farkı
// zemini, not/hata rozeti, toplu seçim kutusu ve seçici liste durumları. DOM ve Solid yok; yalnızca diff-model, strings-diff ve türler içe aktarılır.
import { tsd } from "../../app/strings-diff";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import type { LineKind } from "../../ipc/bindings/LineKind";
import type { SideState } from "./diff-controller";
import { categoryOf, itemLabel, kindLabel, problemText, selectAllState } from "./diff-model";

/** Fark listesi satır yüksekliği (piksel). */
export const ROW_HEIGHT = 26;
/** Satır farkı satır yüksekliği (piksel). */
export const LINE_HEIGHT = 18;

/** Satırın durum metni: sorunlu öğede sorun, değilse kategori. */
export function statusText(i: DiffItem): string {
  return problemText(i) ?? tsd.category[categoryOf(i)];
}

/** `role="option"` satırının etiketi ve `aria-selected` değeri (odak; seçim kutusu ayrıca etiketlidir). */
export function rowAria(i: DiffItem, selected: boolean, focused: boolean): { label: string; selected: boolean } {
  const parts = [`${kindLabel(i.kind)} ${itemLabel(i)}`, statusText(i), tsd.risk[i.risk]];
  if (selected) parts.push(tsd.aria.selected);
  if (!i.selectable) parts.push(tsd.aria.notSelectable);
  return { label: parts.join(", "), selected: focused };
}

const LINE_CLASS: Record<LineKind, string> = {
  same: "",
  added: "bg-[color-mix(in_srgb,var(--srv-green)_14%,transparent)]",
  removed: "bg-[color-mix(in_srgb,var(--danger)_14%,transparent)]",
};

/** Satır farkı zemini (renk tek başına bilgi değildir: işaret sütunu ayrıca `+`/`−` taşır). */
export function lineClass(k: LineKind): string {
  return LINE_CLASS[k];
}

/** Not/hata rozeti: hata varsa `error` (sayı hata sayısı), yalnız not varsa `info`; başlıkta önce hatalar. */
export function noteBadge(i: DiffItem): { count: number; tone: "info" | "error"; title: string } | null {
  const title = [...i.errors, ...i.notes].join("\n");
  if (i.errors.length > 0) return { count: i.errors.length, tone: "error", title };
  if (i.notes.length > 0) return { count: i.notes.length, tone: "info", title };
  return null;
}

/**
 * Üç durumlu toplu seçim kutusu ("Tümü"/"Görünenler"). Seçilebilir öğe yoksa devre dışı ve işaretsizdir; `next`,
 * tıklamanın isteyeceği değerdir (tümü seçiliyse kaldır, değilse seç).
 */
export function bulkBox(
  sel: ReadonlySet<string>,
  items: readonly DiffItem[],
): { checked: boolean; indeterminate: boolean; disabled: boolean; next: boolean } {
  if (!items.some((i) => i.selectable)) return { checked: false, indeterminate: false, disabled: true, next: true };
  const st = selectAllState(sel, items);
  return { checked: st === "all", indeterminate: st === "some", disabled: false, next: st !== "all" };
}

type ListState = { loading: boolean; reason: string | null };

/**
 * Veritabanı ve şema seçicilerinin liste başına durumu. Denetleyicinin `loadingLists` bayrağı taraf başınadır; hangi
 * listenin yüklendiği eksik listeden çıkarılır. `reason` devre dışılığın gerekçesidir (`aria-describedby`).
 */
export function pickerLists(s: SideState): { databases: ListState; schemas: ListState } {
  const databases: ListState = {
    loading: s.loadingLists && s.serverId !== null && s.databases === null,
    reason: s.serverId === null ? tsd.pickerReason.server : null,
  };
  const schemas: ListState = {
    loading: s.loadingLists && s.databaseOid !== null && s.schemas === null,
    reason: s.databaseOid === null ? tsd.pickerReason.database : null,
  };
  return { databases, schemas };
}
