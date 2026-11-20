// Partition formunun saf modeli (tembel parça; Solid içe aktarmaz): kip geçişi (Karar 6), sınır iskeleti ve düzenleme
// yardımcıları, alt bölümleme anahtarı işlemleri ve alter'da sınır değişimi denetimi. SQL üretmez: sınır değerleri ve
// ifadeler olduğu gibi gider; arka uç yetkilidir.
import type { BoundValue } from "../../ipc/bindings/BoundValue";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PartitionBound } from "../../ipc/bindings/PartitionBound";
import type { PartitionDef } from "../../ipc/bindings/PartitionDef";
import type { PartitionKey } from "../../ipc/bindings/PartitionKey";
import type { PartitionMode } from "../../ipc/bindings/PartitionMode";
import type { PartitionStrategy } from "../../ipc/bindings/PartitionStrategy";

export function asPartition(d: ObjectDef | null): (PartitionDef & { kind: "partition" }) | null {
  return d && d.kind === "partition" ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind` korunur). Tür uyuşmazlığında aynı nesne. */
export function partitionPatch(d: ObjectDef, patch: Partial<PartitionDef>): ObjectDef {
  return d.kind === "partition" ? { ...d, ...patch, kind: "partition" } : d;
}

/** Stratejinin boş iskeleti (spec §11.4 `defaults` ile aynı): range → keyCount boş `value`; list → []; hash → 1/0. */
export function emptyBound(strategy: PartitionStrategy, keyCount: number): PartitionBound {
  if (strategy === "list") return { kind: "list", values: [] };
  if (strategy === "hash") return { kind: "hash", modulus: 1, remainder: 0 };
  const cells = (): BoundValue[] => Array.from({ length: Math.max(0, keyCount) }, () => ({ kind: "value", sql: "" }));
  return { kind: "range", from: cells(), to: cells() };
}

/**
 * Karar 6 kip geçişi: attach → partitionBy/tablespace/owner/comment = null (arka uç yok sayar; taslak temiz kalsın);
 * create → name "", schema = parentSchema. Aynı kip → aynı nesne.
 */
export function modePatch(d: PartitionDef, mode: PartitionMode): PartitionDef {
  if (d.mode === mode) return d;
  if (mode === "attach") return { ...d, mode, partitionBy: null, tablespace: null, owner: null, comment: null };
  return { ...d, mode, name: "", schema: d.parentSchema };
}

/** DEFAULT aç/kapa (kapatınca stratejinin boş iskeleti). */
export function defaultPatch(d: PartitionDef, on: boolean): PartitionDef {
  if (on) return d.bound.kind === "default" ? d : { ...d, bound: { kind: "default" } };
  return d.bound.kind === "default" ? { ...d, bound: emptyBound(d.strategy, d.keyCount) } : d;
}

export type BoundSide = "from" | "to";

/** Range hücresini değiştirir; indeks dışı ya da range değilse aynı nesne. */
function rangeCell(d: PartitionDef, side: BoundSide, i: number, f: (c: BoundValue) => BoundValue): PartitionDef {
  const b = d.bound;
  if (b.kind !== "range") return d;
  const cells = b[side];
  if (!Number.isInteger(i) || i < 0 || i >= cells.length) return d;
  return { ...d, bound: { ...b, [side]: cells.map((c, n) => (n === i ? f(c) : c)) } };
}

/**
 * Range hücresi tür değişimi: "value" seçilirse önceki metin (aynı hücre value idiyse) korunur, aksi hâlde "";
 * anahtar sözcükte (MINVALUE/MAXVALUE) metin atılır. Range değilse aynı nesne.
 */
export function rangeKindPatch(d: PartitionDef, side: BoundSide, i: number, kind: BoundValue["kind"]): PartitionDef {
  const cur = d.bound.kind === "range" ? d.bound[side][i] : undefined;
  if (!cur || cur.kind === kind) return d;
  return rangeCell(d, side, i, () => (kind === "value" ? { kind: "value", sql: "" } : { kind }));
}

/** Range hücresinin metni (olduğu gibi, kırpılmaz). */
export function rangeValuePatch(d: PartitionDef, side: BoundSide, i: number, sql: string): PartitionDef {
  return rangeCell(d, side, i, () => ({ kind: "value", sql }));
}

/** List: sona boş değer ekler. */
export function listAdd(d: PartitionDef): PartitionDef {
  return d.bound.kind === "list" ? { ...d, bound: { ...d.bound, values: [...d.bound.values, ""] } } : d;
}

/** List: satır siler; indeks dışı no-op. */
export function listRemove(d: PartitionDef, i: number): PartitionDef {
  const b = d.bound;
  if (b.kind !== "list" || !Number.isInteger(i) || i < 0 || i >= b.values.length) return d;
  return { ...d, bound: { ...b, values: b.values.filter((_, n) => n !== i) } };
}

/** List: değer metnini olduğu gibi yazar; indeks dışı no-op. */
export function listValuePatch(d: PartitionDef, i: number, sql: string): PartitionDef {
  const b = d.bound;
  if (b.kind !== "list" || !Number.isInteger(i) || i < 0 || i >= b.values.length) return d;
  return { ...d, bound: { ...b, values: b.values.map((v, n) => (n === i ? sql : v)) } };
}

const U32_MAX = 4294967295;

/** Sayı metni: /^\d+$/ ve ≤ 2^32−1 değilse aynı nesne (geçersiz metin alanı değiştirmez). */
export function hashPatch(d: PartitionDef, field: "modulus" | "remainder", text: string): PartitionDef {
  if (d.bound.kind !== "hash" || !/^\d+$/.test(text)) return d;
  const n = Number(text);
  if (!Number.isSafeInteger(n) || n > U32_MAX) return d;
  return { ...d, bound: { ...d.bound, [field]: n } };
}

/** Sınır yapısal olarak eşit mi (alan sırasından bağımsız). */
function sameBound(a: PartitionBound, b: PartitionBound): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "range" && b.kind === "range") {
    const eq = (x: BoundValue[], y: BoundValue[]) =>
      x.length === y.length && x.every((c, i) => c.kind === y[i].kind && (c.kind !== "value" || c.sql === (y[i] as { sql: string }).sql));
    return eq(a.from, b.from) && eq(a.to, b.to);
  }
  if (a.kind === "list" && b.kind === "list") return a.values.length === b.values.length && a.values.every((v, i) => v === b.values[i]);
  if (a.kind === "hash" && b.kind === "hash") return a.modulus === b.modulus && a.remainder === b.remainder;
  return true; // default
}

/** Alter'da sınır değişti mi (yapısal eşitlik). Create'te (original null) false. */
export function boundChanged(original: PartitionDef | null, d: PartitionDef): boolean {
  return original !== null && !sameBound(original.bound, d.bound);
}

/** Attach seçimi: lookup öğesinden (value = ad, detail = şema) schema/name. */
export function attachPick(d: PartitionDef, name: string, schema: string): PartitionDef {
  return { ...d, schema, name };
}

// ---- alt bölümleme (yalnız create + mode create; küçük, bağımsız düzenleyici)

/** Alt bölümleme stratejisi: null → yok; seçilince anahtar listesi korunur, yoksa tek boş kolon anahtarı. */
export function subStrategyPatch(d: PartitionDef, s: PartitionStrategy | null): PartitionDef {
  if (s === null) return d.partitionBy === null ? d : { ...d, partitionBy: null };
  return { ...d, partitionBy: { strategy: s, keys: d.partitionBy?.keys ?? [{ kind: "column", name: "" }] } };
}

/** Anahtar listesini yazar (RowList ekle/sil/taşı sonucu); alt bölümleme yoksa aynı nesne. */
export function subKeysPatch(d: PartitionDef, keys: PartitionKey[]): PartitionDef {
  return d.partitionBy ? { ...d, partitionBy: { ...d.partitionBy, keys } } : d;
}

/** Anahtarı yazar (tür değişimi ya da metin); indeks dışı no-op. */
export function subKeyPatch(d: PartitionDef, j: number, key: PartitionKey): PartitionDef {
  const p = d.partitionBy;
  if (!p || !Number.isInteger(j) || j < 0 || j >= p.keys.length) return d;
  return { ...d, partitionBy: { ...p, keys: p.keys.map((k, n) => (n === j ? key : k)) } };
}

/** Anahtar türünü değiştirir; metin taşınır (ad ↔ sql). */
export function subKeyKind(k: PartitionKey, kind: PartitionKey["kind"]): PartitionKey {
  if (k.kind === kind) return k;
  return k.kind === "column" ? { kind: "expression", sql: k.name } : { kind: "column", name: k.sql };
}
