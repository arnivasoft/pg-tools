// Type formunun saf modeli (tembel parça; Solid içe aktarmaz): daraltma/güncelleme, varyant değişimi (yalnız create),
// enum değerleri ve composite öznitelikleri için satır işlemleri, DataLoss özeti ve salt okunur varyant tespiti.
// Kurallar arka ucun (`pgt-ddl` `build/pgtype.rs`) eşlemesini aynalar (Karar 6): değerler/öznitelikler `original` ile
// eşlenir (`null` = yeni). Enum: yüklü değer silinemez, yüklülerin göreli sırası değişmez (yeni değer her yere girebilir;
// arka uç konumu taslak sırasından `AFTER`/`BEFORE` olarak türetir). Composite: yüklü öznitelik taşınamaz, yeni öznitelik
// yalnız sona (PG `ADD ATTRIBUTE` sona ekler). SQL üretmez; diğer kurallar arka ucun alan hatasıdır.
// enumAdd/enumRemove/enumMove/attrAdd/attrRemove/attrMove/attrMoveAllowed: RowList kapılarının test edilmiş başvurusu (form çağırmaz).
import type { CompositeAttr } from "../../ipc/bindings/CompositeAttr";
import type { EnumValue } from "../../ipc/bindings/EnumValue";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TypeBody } from "../../ipc/bindings/TypeBody";
import type { TypeDef } from "../../ipc/bindings/TypeDef";
import { canMove, insertRow, moveRow, removeRow, type MoveDir } from "./row-list-model";

export type TypeDraft = TypeDef & { kind: "type" };
export type TypeVariant = TypeBody["variant"];
/** Varyant değişiminde bırakılan gövdeler (create'te gidiş-dönüşte veri kaybolmasın). */
export type VariantStash = Partial<Record<TypeVariant, TypeBody>>;

export function asType(d: ObjectDef | null): TypeDraft | null {
  return d && d.kind === "type" ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind`, `editable`, `unsupported` korunur). Tür uyuşmazlığında aynı nesne. */
export function typePatch(d: ObjectDef, patch: Partial<TypeDef>): ObjectDef {
  return d.kind === "type" ? { ...d, ...patch, kind: "type" } : d;
}

export function variantOf(d: TypeDef): TypeVariant {
  return d.body.variant;
}

function emptyBody(v: TypeVariant): TypeBody {
  switch (v) {
    case "enum":
      return { variant: "enum", values: [] };
    case "composite":
      return { variant: "composite", attributes: [] };
    case "range":
      return { variant: "range", subtype: "", subtypeOpclass: null, collation: null, canonical: null, subtypeDiff: null, multirangeTypeName: null };
  }
}

/**
 * Varyant değişimi — yalnız create (`original === null`); alter'da tür değiştirilemez (arka uç `body` hatası) ve işlem
 * aynı nesneyi döndürür. Bırakılan gövde `stash`'e yazılır, hedef varyantın yedeği varsa geri yüklenir.
 */
export function switchVariant<T extends TypeDef>(d: T, v: TypeVariant, original: TypeDef | null, stash?: VariantStash): T {
  if (original !== null || d.body.variant === v) return d;
  if (stash) stash[d.body.variant] = d.body;
  return { ...d, body: stash?.[v] ?? emptyBody(v) };
}

// ---- Enum ----

const enumValues = (d: TypeDef): EnumValue[] | null => (d.body.variant === "enum" ? d.body.values : null);
const withValues = <T extends TypeDef>(d: T, values: EnumValue[]): T => ({ ...d, body: { variant: "enum", values } });
const isLoaded = (x: { original: string | null }) => x.original !== null;

/** Yeni değeri `at` konumuna ekler (`original = null`; konum [0, uzunluk] aralığına sıkıştırılır). */
export function enumAdd<T extends TypeDef>(d: T, at: number, label: string): T {
  const vs = enumValues(d);
  return vs ? withValues(d, insertRow(vs, at, { label, original: null })) : d;
}

/** Ekleme konumu: imleçteki (odaklı) satırın altı; imleç yoksa ya da liste dışındaysa sona. */
export function enumAddIndex(length: number, cursor: number | null): number {
  return cursor !== null && Number.isInteger(cursor) && cursor >= 0 && cursor < length ? cursor + 1 : length;
}

/** Etiketi değiştirir; `original` korunur (yüklü değerde `RENAME VALUE`). Sınır dışı → aynı nesne. */
export function enumRename<T extends TypeDef>(d: T, i: number, label: string): T {
  const vs = enumValues(d);
  if (!vs || !Number.isInteger(i) || i < 0 || i >= vs.length) return d;
  return withValues(d, vs.map((v, k) => (k === i ? { ...v, label } : v)));
}

/** Yalnız yeni değer (`original = null`) silinebilir; PostgreSQL enum değerini silemez. */
export function enumRemoveAllowed(d: TypeDef, i: number): boolean {
  const v = enumValues(d)?.[i];
  return v !== undefined && !isLoaded(v);
}

export function enumRemove<T extends TypeDef>(d: T, i: number): T {
  const vs = enumValues(d);
  return vs && enumRemoveAllowed(d, i) ? withValues(d, removeRow(vs, i)) : d;
}

/**
 * Taşıma: komşu liste içinde ve iki satırdan en az biri yeni. İki yüklü değerin yer değiştirmesi yüklülerin göreli
 * sırasını değiştirirdi (arka uç `body.values` hatası) — reddedilir. Yüklü–yeni takası yalnız yeni değerin konumunu değiştirir.
 */
export function enumMoveAllowed(d: TypeDef, i: number, dir: MoveDir): boolean {
  const vs = enumValues(d);
  if (!vs || !canMove(i, dir, vs.length)) return false;
  return !(isLoaded(vs[i]!) && isLoaded(vs[i + dir]!));
}

export function enumMove<T extends TypeDef>(d: T, i: number, dir: MoveDir): T {
  const vs = enumValues(d);
  return vs && enumMoveAllowed(d, i, dir) ? withValues(d, moveRow(vs, i, dir)) : d;
}

/** Taslakta yeni değer var mı (PG < 12'de `ADD VALUE` transaction dışıdır notu). */
export function enumHasNew(d: TypeDef): boolean {
  return enumValues(d)?.some((v) => !isLoaded(v)) ?? false;
}

// ---- Composite ----

const attrs = (d: TypeDef): CompositeAttr[] | null => (d.body.variant === "composite" ? d.body.attributes : null);
const withAttrs = <T extends TypeDef>(d: T, attributes: CompositeAttr[]): T => ({ ...d, body: { variant: "composite", attributes } });

export function newAttr(): CompositeAttr {
  return { name: "", typeName: "", collation: null, original: null };
}

/** Yeni özniteliği yalnız SONA ekler (`original = null`). */
export function attrAdd<T extends TypeDef>(d: T, attr: CompositeAttr): T {
  const as = attrs(d);
  return as ? withAttrs(d, [...as, { ...attr, original: null }]) : d;
}

/** Öznitelik alanlarını günceller; `original` korunur. Sınır dışı → aynı nesne. */
export function attrUpdate<T extends TypeDef>(d: T, i: number, patch: Partial<Omit<CompositeAttr, "original">>): T {
  const as = attrs(d);
  if (!as || !Number.isInteger(i) || i < 0 || i >= as.length) return d;
  return withAttrs(d, as.map((a, k) => (k === i ? { ...a, ...patch, original: a.original } : a)));
}

/** Silme: yüklü öznitelikte `dataLoss` (DROP ATTRIBUTE, tipi kullanan tablolardaki veri kaybolur). */
export function attrRemove<T extends TypeDef>(d: T, i: number): { def: T; dataLoss: boolean } {
  const as = attrs(d);
  const a = as?.[i];
  if (!as || a === undefined) return { def: d, dataLoss: false };
  return { def: withAttrs(d, removeRow(as, i)), dataLoss: isLoaded(a) };
}

/** Taşıma yalnız iki yeni öznitelik arasında: yüklü öznitelik taşınamaz ve yeni öznitelik yüklünün önüne geçemez. */
export function attrMoveAllowed(d: TypeDef, i: number, dir: MoveDir): boolean {
  const as = attrs(d);
  return !!as && canMove(i, dir, as.length, (k) => isLoaded(as[k]!));
}

export function attrMove<T extends TypeDef>(d: T, i: number, dir: MoveDir): T {
  const as = attrs(d);
  return as && attrMoveAllowed(d, i, dir) ? withAttrs(d, moveRow(as, i, dir)) : d;
}

/** Satırın kilidi (RowList `moveLocked`): yüklü öznitelik. */
export function attrLoaded(d: TypeDef, i: number): boolean {
  const a = attrs(d)?.[i];
  return a !== undefined && isLoaded(a);
}

/**
 * Alter DataLoss özeti: silinen yüklü öznitelikler (`DROP ATTRIBUTE`) ve tipi ya da collation'ı değişen yüklü
 * öznitelikler (`ALTER ATTRIBUTE … TYPE`). İstemci tipleri metin olarak karşılaştırır (`int` ↔ `int4` gibi eşdeğerlikleri
 * arka uç ayıklar; not gereksiz görünebilir, zararsız). Yeniden adlandırma kayıp değildir.
 */
export function compositeLoss(original: TypeDef | null, d: TypeDef): { dropped: string[]; retyped: string[] } {
  const old = original ? attrs(original) : null;
  const cur = attrs(d);
  if (!old || !cur) return { dropped: [], retyped: [] };
  const dropped = old.filter((o) => !cur.some((a) => a.original === o.name)).map((o) => o.name);
  const retyped: string[] = [];
  for (const a of cur) {
    const o = a.original === null ? undefined : old.find((x) => x.name === a.original);
    if (o && (o.typeName.trim() !== a.typeName.trim() || o.collation !== a.collation)) retyped.push(a.name);
  }
  return { dropped, retyped };
}

/**
 * Silinen yüklü özniteliği geri alır: yüklü tanımdaki hâliyle, yüklü sıradaki yerine (kendisinden önce gelen yüklü
 * özniteliklerin ardına; yeni öznitelikler sonda kalır). Zaten taslaktaysa ya da yüklü değilse aynı nesne.
 */
export function attrRestore<T extends TypeDef>(d: T, original: TypeDef | null, name: string): T {
  const as = attrs(d);
  const old = original ? attrs(original) : null;
  if (!as || !old) return d;
  const oi = old.findIndex((o) => o.name === name);
  if (oi < 0 || as.some((a) => a.original === name)) return d;
  const before = new Set(old.slice(0, oi).map((o) => o.name));
  let at = 0;
  as.forEach((a, k) => {
    if (a.original !== null && before.has(a.original)) at = k + 1;
  });
  return withAttrs(d, insertRow(as, at, { ...old[oi]! }));
}

// ---- Salt okunur varyant ----

/**
 * Yükleyici base/array/pseudo/shell/multirange tipleri boş `composite` gövde + `unsupported` + `editable = false` olarak
 * döndürür (spec §8.1): form bunları boş bir composite düzenleyicisi olarak değil, "bu tip türü düzenlenemez" diye gösterir.
 */
export function isReadOnlyVariant(d: TypeDef): boolean {
  return !d.editable && d.body.variant === "composite" && d.body.attributes.length === 0;
}
