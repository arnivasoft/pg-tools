// Şema karşılaştırma görünümlerinin saf yardımcıları (Plan 30): satır erişilebilirlik metni, risk tonu, satır farkı
// zemini ve not/hata rozeti. DOM ve Solid yok; yalnızca diff-model, strings-diff ve türler içe aktarılır.
import { tsd } from "../../app/strings-diff";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import type { LineKind } from "../../ipc/bindings/LineKind";
import type { Risk } from "../../ipc/bindings/Risk";
import { categoryOf, itemLabel, kindLabel, problemText } from "./diff-model";

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

export function riskTone(r: Risk): "muted" | "warn" | "danger" {
  return r === "data_loss" ? "danger" : r === "destructive" ? "warn" : "muted";
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
