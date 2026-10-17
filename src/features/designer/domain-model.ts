// Domain formunun saf modeli (tembel parça; Solid içe aktarmaz): daraltma/güncelleme ve CHECK kısıtı satır işlemleri.
// Kurallar arka ucun (`pgt-ddl` `build/domain.rs`) eşlemesini aynalar (Karar 6): kısıtlar `original` (yüklü ad) ile
// eşlenir (`null` = yeni). Alter'da eklenen kısıtın adı zorunludur (yerel ön uyarı; asıl hata arka uçtan); `NOT VALID`
// yalnız eklenen (alter'da yeni ya da ifadesi değişen) kısıtta; `validate` yalnız yüklü, doğrulanmamış ve ifadesi
// değişmeyen kısıtta. İfade karşılaştırması metin eşitliğidir (arka uç da yüklü `pg_get_constraintdef` metnine karşı
// metin eşitliği kullanır). SQL üretmez; diğer kurallar arka ucun alan hatasıdır.
import type { DomainCheck } from "../../ipc/bindings/DomainCheck";
import type { DomainDef } from "../../ipc/bindings/DomainDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import { removeRow } from "./row-list-model";

export type DomainDraft = DomainDef & { kind: "domain" };
export type DomainMode = "create" | "alter";

export function asDomain(d: ObjectDef | null): DomainDraft | null {
  return d && d.kind === "domain" ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind`, `editable`, `unsupported` korunur). Tür uyuşmazlığında aynı nesne. */
export function domainPatch(d: ObjectDef, patch: Partial<DomainDef>): ObjectDef {
  return d.kind === "domain" ? { ...d, ...patch, kind: "domain" } : d;
}

/** Yeni CHECK kısıtı (`original = null`). */
export function newCheck(): DomainCheck {
  return { name: "", expression: "", notValid: false, validated: false, original: null, validate: false };
}

const inRange = (d: DomainDef, i: number) => Number.isInteger(i) && i >= 0 && i < d.constraints.length;

/** Sona yeni kısıt ekler. */
export function checkAdd<T extends DomainDef>(d: T): T {
  return { ...d, constraints: [...d.constraints, newCheck()] };
}

/** Kısıtı siler (yüklü kısıtta arka uç `DROP CONSTRAINT` — Destructive). Sınır dışı → aynı nesne. */
export function checkRemove<T extends DomainDef>(d: T, i: number): T {
  return inRange(d, i) ? { ...d, constraints: removeRow(d.constraints, i) } : d;
}

/** Ad değişimi; `original` korunur (yüklüde `RENAME CONSTRAINT`). Boş ya da yalnız boşluk → `null` (create'te adı sunucu verir). */
export function checkRename<T extends DomainDef>(d: T, i: number, name: string): T {
  if (!inRange(d, i)) return d;
  const n = name.trim() === "" ? null : name;
  return { ...d, constraints: d.constraints.map((c, k) => (k === i ? { ...c, name: n } : c)) };
}

/** Alter'da adsız kısıt: arka uç ad ister (sonraki düzenlemede eşlenebilmesi için). Create'te ad isteğe bağlıdır. */
export function needsName(mode: DomainMode, c: DomainCheck): boolean {
  return mode === "alter" && (c.name === null || c.name === "");
}

function loadedOf(c: DomainCheck, original: DomainDef | null): DomainCheck | null {
  if (!original || c.original === null) return null;
  return original.constraints.find((o) => o.name === c.original) ?? null;
}

/** "Doğrula" (`VALIDATE CONSTRAINT`): yalnız alter'da yüklü, doğrulanmamış ve ifadesi değişmemiş kısıt. */
export function canValidate(c: DomainCheck, original: DomainDef | null): boolean {
  const o = loadedOf(c, original);
  return o !== null && !o.validated && o.expression === c.expression;
}

/** `NOT VALID`: yalnız alter'da eklenen kısıt (yeni ya da ifadesi değişen; create'te PG desteklemez). */
export function notValidAllowed(c: DomainCheck, original: DomainDef | null): boolean {
  if (!original) return false;
  if (c.original === null) return true;
  const o = loadedOf(c, original);
  return o !== null && o.expression !== c.expression;
}

/**
 * Kısıt alanlarını günceller (`original` korunur); izin kalkan bayraklar sıfırlanır (ör. ifade yüklü hâline dönünce
 * `notValid`, ifade değişince `validate`) — gizlenen kutu arka uç hatasına yol açmasın. Sınır dışı → aynı nesne.
 */
export function checkUpdate<T extends DomainDef>(
  d: T,
  i: number,
  patch: Partial<Omit<DomainCheck, "original" | "validated">>,
  original: DomainDef | null,
): T {
  if (!inRange(d, i)) return d;
  return {
    ...d,
    constraints: d.constraints.map((c, k) => {
      if (k !== i) return c;
      const n: DomainCheck = { ...c, ...patch, original: c.original, validated: c.validated };
      if (n.notValid && !notValidAllowed(n, original)) n.notValid = false;
      if (n.validate && !canValidate(n, original)) n.validate = false;
      return n;
    }),
  };
}

/** Taslakta eşi (`c.original === ad`) kalmayan yüklü kısıtların adları, yüklü sırada (silinecekler). Create → boş. */
export function removedChecks(original: DomainDef | null, d: DomainDef): string[] {
  if (!original) return [];
  const kept = new Set(d.constraints.map((c) => c.original).filter((n): n is string => n !== null));
  return original.constraints.map((o) => o.name).filter((n): n is string => n !== null && !kept.has(n));
}

/** Silinen yüklü kısıtı yüklü hâliyle sona geri ekler ("Geri al"). Zaten eşlenmişse ya da bulunamazsa aynı nesne. */
export function checkRestore<T extends DomainDef>(d: T, original: DomainDef | null, name: string): T {
  const o = original?.constraints.find((c) => c.name === name);
  if (!o || d.constraints.some((c) => c.original === name)) return d;
  return { ...d, constraints: [...d.constraints, { ...o, original: name }] };
}
