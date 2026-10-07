// Table kısıtlarının saf modeli: oluşturma/tür değişimi, kolon seçicileri, FK çiftleri, DEFERRABLE / NOT VALID / VALIDATE
// kuralları, sil/geri al, sarkan referanslar ve desteklenmeyen girdilerin sınıflandırması. İstemci SQL üretmez;
// hepsi değişmezdir (girdi nesnesi değiştirilmez, sınır dışı indeks → aynı nesne). `oid` yalnızca yükleme kimliğidir:
// istemci asla üretmez/değiştirmez.
import { ttbl } from "../../app/strings-table";
import type { ConstraintDef } from "../../ipc/bindings/ConstraintDef";
import type { ConstraintKind } from "../../ipc/bindings/ConstraintKind";
import type { FkAction } from "../../ipc/bindings/FkAction";
import type { FkMatch } from "../../ipc/bindings/FkMatch";
import type { TableDef } from "../../ipc/bindings/TableDef";
import { selfRefs } from "./table-model";

export type ConstraintType = ConstraintKind["type"];
export type ColumnListName = "columns" | "include";

const inRange = (i: number, len: number) => Number.isInteger(i) && i >= 0 && i < len;

// ---- kısıt işlemleri

/** Yeni kısıt (oid null, name null, bayraklar false). FK hedefi varsayılan olarak taslağın şeması. */
export function newConstraint(type: ConstraintType, d: TableDef): ConstraintDef {
  const base = { oid: null, name: null, deferrable: false, initiallyDeferred: false, notValid: false };
  switch (type) {
    case "primary_key": return { ...base, kind: { type, columns: [], include: [] } };
    case "unique": return { ...base, kind: { type, columns: [], include: [], nullsNotDistinct: false } };
    case "check": return { ...base, kind: { type, expr: "", noInherit: false } };
    case "foreign_key":
      return { ...base, kind: { type, columns: [""], refSchema: d.schema, refTable: "", refColumns: [""], matchType: "simple", onUpdate: "no_action", onDelete: "no_action" } };
  }
}

export function addConstraint(d: TableDef, type: ConstraintType): TableDef {
  return { ...d, constraints: [...d.constraints, newConstraint(type, d)] };
}

export function updateConstraint(d: TableDef, i: number, f: (c: ConstraintDef) => ConstraintDef): TableDef {
  if (!inRange(i, d.constraints.length)) return d;
  const next = f(d.constraints[i]!);
  if (next === d.constraints[i]) return d;
  return { ...d, constraints: d.constraints.map((c, k) => (k === i ? next : c)) };
}

export function removeConstraint(d: TableDef, i: number): TableDef {
  if (!inRange(i, d.constraints.length)) return d;
  return { ...d, constraints: d.constraints.filter((_, k) => k !== i) };
}

export function supportsDeferrable(type: ConstraintType): boolean {
  return type === "primary_key" || type === "unique" || type === "foreign_key";
}

export function supportsNotValid(type: ConstraintType): boolean {
  return type === "foreign_key" || type === "check";
}

/** Yalnızca yeni kısıtta (oid null) tür değişimi: ad korunur; desteklenmeyen bayraklar false. Yüklü kısıtta c. */
export function switchConstraintType(c: ConstraintDef, type: ConstraintType, d: TableDef): ConstraintDef {
  if (c.oid !== null || c.kind.type === type) return c;
  if (type === "primary_key" && d.constraints.some((o) => o !== c && o.kind.type === "primary_key")) return c;
  const fresh = newConstraint(type, d);
  const deferrable = supportsDeferrable(type) && c.deferrable;
  return {
    ...fresh,
    name: c.name,
    deferrable,
    initiallyDeferred: deferrable && c.initiallyDeferred,
    notValid: supportsNotValid(type) && c.notValid,
  };
}

/** `excludingIndex`: düzenlenen kısıt sayılmaz (kendi türünü PK'ya çevirirken). */
export function canAddPrimaryKey(d: TableDef, excludingIndex?: number): boolean {
  return !d.constraints.some((c, i) => i !== excludingIndex && c.kind.type === "primary_key");
}

/** Silinen yüklü kısıt geri alınabilir mi: taslakta yok, özgünde var ve (PK ise) taslakta başka PK yok. */
export function canRestoreConstraint(d: TableDef, original: TableDef, oid: number): boolean {
  if (d.constraints.some((c) => c.oid === oid)) return false;
  const o = original.constraints.find((c) => c.oid === oid);
  if (!o) return false;
  return o.kind.type !== "primary_key" || canAddPrimaryKey(d);
}

/** DEFERRABLE kapatılınca initiallyDeferred da false. */
export function withDeferrable(c: ConstraintDef, v: boolean): ConstraintDef {
  if (!supportsDeferrable(c.kind.type)) return c;
  if (c.deferrable === v) return c;
  return { ...c, deferrable: v, initiallyDeferred: v ? c.initiallyDeferred : false };
}

/** deferrable false iken etkisiz (c aynen). */
export function withInitiallyDeferred(c: ConstraintDef, v: boolean): ConstraintDef {
  return c.deferrable && c.initiallyDeferred !== v ? { ...c, initiallyDeferred: v } : c;
}

/**
 * NOT VALID denetimi: "add" → yalnızca alter'da, yeni (oid null) FK/CHECK; "validate" → yüklü ve özgün tanımda
 * notValid=true olan kısıt ("Doğrula" kutusu notValid'i false yapar); "none" → gösterilmez.
 * Not (Task 6 etiketi): yapısı değişmiş yüklü kısıtta notValid=false, arka uçta "yeniden doğrulanmış ekle" anlamına gelir.
 */
export function notValidMode(mode: "create" | "alter", c: ConstraintDef, original: ConstraintDef | null): "add" | "validate" | "none" {
  const type = c.kind.type;
  if (!supportsNotValid(type)) return "none";
  if (c.oid === null) return mode === "alter" ? "add" : "none"; // create'te NOT VALID hata (Plan 12)
  return original?.notValid ? "validate" : "none"; // doğrulanmış kısıt "doğrulanmamış" yapılamaz
}

/** Yüklenen tanımdaki eşi (oid ile); yeni kısıt → null. */
export function originalConstraint(original: TableDef | null, c: ConstraintDef): ConstraintDef | null {
  if (!original || c.oid === null) return null;
  return original.constraints.find((o) => o.oid === c.oid) ?? null;
}

/** Taslakta olmayan yüklü kısıtlar (özgün sırayla). */
export function removedConstraints(original: TableDef | null, d: TableDef): ConstraintDef[] {
  if (!original) return [];
  return original.constraints.filter((o) => o.oid !== null && !d.constraints.some((c) => c.oid === o.oid));
}

/**
 * Özgün kolon adı → taslaktaki güncel adı (`attnum` eşlemesiyle; yeniden adlandırılmış yüklü kolonlar). Taslakta
 * olmayan (silinmiş) kolonlar haritada yoktur.
 */
export function currentColumnNames(original: TableDef, d: TableDef): Map<string, string> {
  const names = new Map<string, string>();
  for (const oc of original.columns) {
    const now = oc.attnum === null ? undefined : d.columns.find((x) => x.attnum === oc.attnum);
    if (now) names.set(oc.name, now.name);
  }
  return names;
}

const isSelfIn = (t: TableDef, k: ConstraintKind) => k.type === "foreign_key" && k.refSchema === t.schema && k.refTable === t.name;

/**
 * Yüklü kısıtı (özgün kopyası) özgün sıradaki yerine (önündeki yüklü kısıt sayısı) geri koyar. Kolon adları `attnum`
 * ile taslağın GÜNCEL adlarına eşlenir (yeniden adlandırılmış kolona başvuru sarkmasın; kendine başvuran FK'nin hedef
 * kolonları da); silinmiş kolonun adı aynen kalır (sarkan referans uyarısı). Yeniden adlandırma yoksa birebir kopya.
 */
export function restoreConstraint(d: TableDef, original: TableDef, oid: number): TableDef {
  if (d.constraints.some((c) => c.oid === oid)) return d;
  const pos = original.constraints.findIndex((c) => c.oid === oid);
  if (pos < 0) return d;
  const before = new Set(original.constraints.slice(0, pos).map((c) => c.oid));
  const at = d.constraints.filter((c) => c.oid !== null && before.has(c.oid)).length;
  const restored = structuredClone(original.constraints[pos]!);
  const names = currentColumnNames(original, d);
  const map = (a: string[]) => a.map((n) => names.get(n) ?? n);
  const k = restored.kind;
  if (k.type === "primary_key" || k.type === "unique") {
    k.columns = map(k.columns);
    k.include = map(k.include);
  } else if (k.type === "foreign_key") {
    const self = isSelfIn(original, k);
    k.columns = map(k.columns);
    if (self) k.refColumns = map(k.refColumns);
  }
  const constraints = [...d.constraints];
  constraints.splice(at, 0, restored);
  return { ...d, constraints };
}

// ---- PRIMARY KEY üyelerinin NOT NULL'ı

const pkMembers = (d: TableDef) => {
  const out = new Set<string>();
  for (const c of d.constraints) if (c.kind.type === "primary_key") for (const n of c.kind.columns) if (n !== "") out.add(n);
  return out;
};


/**
 * PRIMARY KEY üyesi kolonları notNull=true yapar (arka uç aksi halde hata verir). Yalnızca değişen kolonlar yeni nesne
 * olur; değişen yoksa aynı nesne döner. Ad eşleşmesi çağrı anındaki adlarla yapılır (Task 5/6 ilgili düzenleme
 * sonrasında çağırır).
 */
export function withPrimaryKeyNotNull(d: TableDef): TableDef {
  const members = pkMembers(d);
  if (!d.columns.some((col) => !col.notNull && members.has(col.name))) return d;
  return { ...d, columns: d.columns.map((col) => (!col.notNull && members.has(col.name) ? { ...col, notNull: true } : col)) };
}

/**
 * PRIMARY KEY'in zorladığı NOT NULL kaydı (formun düzenleyicisi tutar): yüklü kolonlar `attnum`, yeni kolonlar yeni
 * kolonlar arasındaki sıralarıyla (yeni kolonlar yalnızca sona eklenir; sıraları silme dışında kaymaz). Zorlanan kolonun
 * zorlamadan önceki değeri tanım gereği `false`'tur.
 */
export interface PkForced {
  loaded: readonly number[];
  fresh: readonly boolean[];
}
export const NO_PK_FORCED: PkForced = { loaded: [], fresh: [] };

/** Yeni kolonların önceki → sonraki sıra eşlemesi: `from[k]` = sonraki k. yeni kolonun önceki sırası (eklenen → -1). */
function alignFresh(before: TableDef, after: TableDef): number[] {
  const b = before.columns.filter((c) => c.attnum === null);
  const a = after.columns.filter((c) => c.attnum === null);
  if (a.length >= b.length) return a.map((_, k) => (k < b.length ? k : -1));
  // Silme: ilk ad farkından başlayarak (b.length - a.length) kolon düşmüştür.
  const gone = b.length - a.length;
  let at = 0;
  while (at < a.length && a[at]!.name === b[at]!.name) at++;
  return a.map((_, k) => (k < at ? k : k + gone));
}

/**
 * Bir düzenlemenin ardından PRIMARY KEY üyelerinin NOT NULL'ını uzlaştırır. Formda PK üyelerinin NOT NULL'ını zorlayan
 * TEK yer budur (editör her düzenlemeden sonra çağırır); çağrı yerleri `withPrimaryKeyNotNull` ile önceden zorlamamalı,
 * aksi halde zorlama kaydedilemez (ör. geri alınan kolon, kimlikten dönen kolon) ve PK'dan çıkınca geri dönmez.
 * - üye olup NOT NULL olmayan kolon NOT NULL yapılır ve "zorlanmış" kaydedilir;
 * - zorlanmış kolon artık üye değilse NOT NULL eski değerine (false) döner; böylece PK'ya ekle-çıkar ya da sil-geri al
 *   "değişiklik yok" planına döner;
 * - kimlik kolonu hep NOT NULL'dır: dokunulmaz ama kaydı korunur (kimlikten çıkıp PK'dan da çıkınca false'a döner).
 * Kullanıcının kendi işaretlediği NOT NULL (üye değilken) zorlanmış sayılmaz. Değişen yoksa `after` aynen döner.
 */
export function reconcilePrimaryKeyNotNull(before: TableDef, after: TableDef, forced: PkForced): { d: TableDef; forced: PkForced } {
  const members = pkMembers(after);
  const from = alignFresh(before, after);
  const loaded = new Set(forced.loaded);
  const fresh: boolean[] = [];
  const nextLoaded: number[] = [];
  let ordinal = 0;
  let changed = false;
  const columns = after.columns.map((col) => {
    let wasForced: boolean;
    if (col.attnum === null) {
      const k = from[ordinal++]!;
      wasForced = k >= 0 && forced.fresh[k] === true;
    } else wasForced = loaded.has(col.attnum);
    let out = col;
    let isForced = false;
    if (col.identity !== null) isForced = wasForced;
    else if (members.has(col.name)) {
      if (!col.notNull) {
        out = { ...col, notNull: true };
        isForced = true;
      } else isForced = wasForced;
    } else if (wasForced && col.notNull) out = { ...col, notNull: false };
    if (out !== col) changed = true;
    if (col.attnum === null) fresh.push(isForced);
    else if (isForced) nextLoaded.push(col.attnum);
    return out;
  });
  return { d: changed ? { ...after, columns } : after, forced: { loaded: nextLoaded, fresh } };
}

// ---- kolon listeleri (PK/UNIQUE columns + include)

/** Salt okunur görünüm (kopyalamaz; değiştirmeyin). FK'da yalnızca "columns"; include ve CHECK → []. */
export function constraintColumns(c: ConstraintDef, list: ColumnListName): readonly string[] {
  const k = c.kind;
  if (k.type === "primary_key" || k.type === "unique") return k[list];
  if (k.type === "foreign_key" && list === "columns") return k.columns;
  return [];
}

function withList(c: ConstraintDef, list: ColumnListName, f: (a: string[]) => string[]): ConstraintDef {
  const k = c.kind;
  if (k.type !== "primary_key" && k.type !== "unique") return c;
  return { ...c, kind: { ...k, [list]: f(k[list]) } as ConstraintKind };
}

/** Boş/yinelenen ad eklenmez (aynı nesne). Yalnızca PK/UNIQUE; FK/CHECK'te daima aynı nesne (FK için çift işlemleri). */
export function addConstraintColumn(c: ConstraintDef, list: ColumnListName, name: string): ConstraintDef {
  if (name === "") return c;
  if (constraintColumns(c, list).includes(name)) return c;
  return withList(c, list, (a) => [...a, name]);
}

export function removeConstraintColumn(c: ConstraintDef, list: ColumnListName, j: number): ConstraintDef {
  if (!inRange(j, constraintColumns(c, list).length)) return c;
  return withList(c, list, (a) => a.filter((_, k) => k !== j));
}

/** Seçilebilir kolonlar: taslaktaki boş olmayan adlardan bu listede (ve PK/UNIQUE'te öteki listede) olmayanlar, taslak sırasıyla. */
export function availableColumns(d: TableDef, c: ConstraintDef, list: ColumnListName): string[] {
  const used = new Set(constraintColumns(c, list));
  if (c.kind.type === "primary_key" || c.kind.type === "unique") for (const n of c.kind[list === "columns" ? "include" : "columns"]) used.add(n);
  const out: string[] = [];
  for (const col of d.columns) if (col.name !== "" && !used.has(col.name) && !out.includes(col.name)) out.push(col.name);
  return out;
}

// ---- FK eşleşmeleri (columns[j] ↔ refColumns[j]; uzunluklar eşit)

function withFk(c: ConstraintDef, f: (k: Extract<ConstraintKind, { type: "foreign_key" }>) => ConstraintKind): ConstraintDef {
  if (c.kind.type !== "foreign_key") return c;
  const kind = f(c.kind);
  return kind === c.kind ? c : { ...c, kind };
}

export function addFkPair(c: ConstraintDef): ConstraintDef {
  return withFk(c, (k) => ({ ...k, columns: [...k.columns, ""], refColumns: [...k.refColumns, ""] }));
}

export function updateFkPair(c: ConstraintDef, j: number, p: { local?: string; ref?: string }): ConstraintDef {
  if (c.kind.type !== "foreign_key" || !inRange(j, c.kind.columns.length)) return c;
  const k0 = c.kind;
  if ((p.local === undefined || p.local === k0.columns[j]) && (p.ref === undefined || p.ref === k0.refColumns[j])) return c;
  return withFk(c, (k) => ({
    ...k,
    columns: p.local === undefined ? k.columns : k.columns.map((n, i) => (i === j ? p.local! : n)),
    refColumns: p.ref === undefined ? k.refColumns : k.refColumns.map((n, i) => (i === j ? p.ref! : n)),
  }));
}

export function removeFkPair(c: ConstraintDef, j: number): ConstraintDef {
  if (c.kind.type !== "foreign_key" || !inRange(j, c.kind.columns.length)) return c;
  return withFk(c, (k) => ({ ...k, columns: k.columns.filter((_, i) => i !== j), refColumns: k.refColumns.filter((_, i) => i !== j) }));
}

/** Hedef şema/tablo değişimi: değişen varsa hedef kolon adları "" yapılır (çift sayısı korunur). */
export function withFkTarget(c: ConstraintDef, p: { refSchema?: string; refTable?: string }): ConstraintDef {
  return withFk(c, (k) => {
    const refSchema = p.refSchema ?? k.refSchema;
    const refTable = p.refTable ?? k.refTable;
    if (refSchema === k.refSchema && refTable === k.refTable) return k;
    return { ...k, refSchema, refTable, refColumns: k.columns.map(() => "") };
  });
}

export function withFkActions(c: ConstraintDef, p: { matchType?: FkMatch; onUpdate?: FkAction; onDelete?: FkAction }): ConstraintDef {
  return withFk(c, (k) => (
    (p.matchType ?? k.matchType) === k.matchType && (p.onUpdate ?? k.onUpdate) === k.onUpdate && (p.onDelete ?? k.onDelete) === k.onDelete ? k : {
    ...k,
    matchType: p.matchType ?? k.matchType,
    onUpdate: p.onUpdate ?? k.onUpdate,
    onDelete: p.onDelete ?? k.onDelete,
  }));
}

// ---- görünüm yardımcıları

/** Kart başlığı: ad ya da "adsız <TÜR>". */
export function constraintLabel(c: ConstraintDef): string {
  return c.name !== null && c.name !== "" ? c.name : ttbl.constraints.unnamed(ttbl.constraints.types[c.kind.type]);
}

/**
 * Taslakta artık olmayan (boş olmayan) kolon başvuruları: PK/UNIQUE columns+include, FK columns (+ kendine-başvuran
 * refColumns). `selfRef`, kendine-başvuru kararını çağıranın konum anlık görüntüsünden verir (yazım sırasında geçici
 * bir tablo adı başka tablonun FK'sini "kendine" çevirmesin); verilmezse mevcut ad/şemaya bakılır.
 */
export function danglingColumns(d: TableDef, c: ConstraintDef, selfRef?: boolean): string[] {
  const have = new Set(d.columns.map((col) => col.name));
  const k = c.kind;
  let names: string[] = [];
  if (k.type === "primary_key" || k.type === "unique") names = [...k.columns, ...k.include];
  else if (k.type === "foreign_key") {
    const self = selfRef ?? (k.refSchema === d.schema && k.refTable === d.name);
    names = self ? [...k.columns, ...k.refColumns] : [...k.columns];
  }
  const out: string[] = [];
  for (const n of names) if (n !== "" && !have.has(n) && !out.includes(n)) out.push(n);
  return out;
}

export type UnsupportedKind = "notNull" | "exclude" | "trigger" | "identity" | "storage" | "typed" | "other";

// Önekler crates/pgt-ddl/src/load.rs etiketleriyle birebir ("NOT NULL ", "EXCLUDE ", "CONSTRAINT TRIGGER ",
// "kimlik dizisi seçenekleri", "WITH (", "OF ").
export function unsupportedKind(entry: string): UnsupportedKind {
  if (entry.startsWith("NOT NULL ")) return "notNull";
  if (entry.startsWith("EXCLUDE ")) return "exclude";
  if (entry.startsWith("CONSTRAINT TRIGGER ")) return "trigger";
  if (entry.startsWith("kimlik dizisi seçenekleri")) return "identity";
  if (entry.startsWith("WITH (")) return "storage";
  if (entry.startsWith("OF ")) return "typed";
  return "other";
}

// ---- kendine-başvuran FK konum anlık görüntüsü (SelfSnap)
//
// SelfSnap: düzenleme başlarken `selfRefs(d)` ile alınan, tabloyu hedefleyen FK'lerin TASLAKTAKİ kısıt indeksleri.
// Tablo adı/şeması yazılırken `renameTable(d, patch, snap)` yalnızca bu indekslere yazar; ad bazlı yeniden tespit
// YAPILMAZ (ara ad başka tablonun FK'sini çalardı). Kısıt ekle/sil/geri al indeksleri kaydırır: aşağıdaki sarmalayıcılar
// taslağı ve anlık görüntüyü birlikte günceller. Kısıt düzenlemeleri dışında (ör. PK/UNIQUE alanları) indeks değişmez.

export type SelfSnap = readonly number[];
export interface WithSnap { d: TableDef; snap: SelfSnap }

/** Yeni kısıt sona eklenir; kendine-başvuru değildir (FK hedefi boş) — hedef `pointFkAtSelf` ile açıkça kaydedilir. */
export function addConstraintSnap(d: TableDef, type: ConstraintType, snap: SelfSnap): WithSnap {
  return { d: addConstraint(d, type), snap };
}

/** i. kısıtı siler; sonrasındaki indeksler bir azalır. */
export function removeConstraintSnap(d: TableDef, i: number, snap: SelfSnap): WithSnap {
  const next = removeConstraint(d, i);
  if (next === d) return { d, snap };
  return { d: next, snap: snap.filter((k) => k !== i).map((k) => (k > i ? k - 1 : k)) };
}

/** Silinen yüklü kısıtı geri koyar; yerleştiği indeks ve sonrası bir artar. Özgün kısıt kendine-başvuruysa kaydedilir. */
export function restoreConstraintSnap(d: TableDef, original: TableDef, oid: number, snap: SelfSnap): WithSnap {
  const next = restoreConstraint(d, original, oid);
  if (next === d) return { d, snap };
  const at = next.constraints.findIndex((c) => c.oid === oid);
  const o = original.constraints.find((c) => c.oid === oid)!;
  const shifted = snap.map((k) => (k >= at ? k + 1 : k));
  const wasSelf = isSelfIn(original, o.kind);
  if (!wasSelf) return { d: next, snap: shifted };
  // Geri alınan FK yüklenen tanımdaki (özgün ad/şema) hedefi taşır; taslağın güncel adına hizalanır.
  const fixed = updateConstraint(next, at, (c) => (c.kind.type === "foreign_key" ? { ...c, kind: { ...c.kind, refSchema: next.schema, refTable: next.name } } : c));
  return { d: fixed, snap: [...shifted, at].sort((a, b) => a - b) };
}

/** i. FK'yi tablonun kendisine yöneltir (hedef taslağın şema/adı, ref adları boşalır) ve anlık görüntüye açıkça kaydeder. */
export function pointFkAtSelf(d: TableDef, i: number, snap: SelfSnap): WithSnap {
  const next = updateConstraint(d, i, (c) => withFkTarget(c, { refSchema: d.schema, refTable: d.name }));
  if (next === d || next.constraints[i]!.kind.type !== "foreign_key") return { d, snap };
  return { d: next, snap: snap.includes(i) ? snap : [...snap, i].sort((a, b) => a - b) };
}

/** i. FK'yi başka bir tabloya yöneltir ve anlık görüntüden çıkarır. */
export function pointFkAtOther(d: TableDef, i: number, target: { refSchema?: string; refTable?: string }, snap: SelfSnap): WithSnap {
  const next = updateConstraint(d, i, (c) => withFkTarget(c, target));
  return { d: next, snap: snap.filter((k) => k !== i) };
}

/**
 * Yeni kısıtın türünü değiştirir (`switchConstraintType`) ve indeksi anlık görüntüden çıkarır: yeni FK'nin hedefi boştur
 * (kendine-başvuru değildir); aksi halde tablo adı yazılırken eski kayıt yeni FK'yi tabloya yöneltirdi.
 */
export function switchConstraintTypeSnap(d: TableDef, i: number, type: ConstraintType, snap: SelfSnap): WithSnap {
  const next = updateConstraint(d, i, (c) => switchConstraintType(c, type, d));
  if (next === d) return { d, snap };
  return { d: next, snap: snap.filter((k) => k !== i) };
}

/**
 * FK hedef şema/tablo yazımı: hedef taslağın kendi şema/adıysa (ad boş değilken) `pointFkAtSelf`, aksi halde
 * `pointFkAtOther`. Hedef değişmediyse taslak ve anlık görüntü aynen döner.
 */
export function withFkTargetSnap(d: TableDef, i: number, p: { refSchema?: string; refTable?: string }, snap: SelfSnap): WithSnap {
  const c = d.constraints[i];
  if (!c || c.kind.type !== "foreign_key") return { d, snap };
  const refSchema = p.refSchema ?? c.kind.refSchema;
  const refTable = p.refTable ?? c.kind.refTable;
  if (refSchema === c.kind.refSchema && refTable === c.kind.refTable) return { d, snap };
  if (refTable !== "" && refSchema === d.schema && refTable === d.name) return pointFkAtSelf(d, i, snap);
  return pointFkAtOther(d, i, { refSchema, refTable }, snap);
}

/**
 * FK hedef kolon önerilerinin kaynağı: hedef tablo boş → "none"; kendine başvuran FK → "draft" (taslak kolon adları;
 * henüz kaydedilmemiş kolonlar da seçilebilsin); aksi halde "lookup" (`table_columns`). FK değilse "none".
 * `selfRef`: kendine-başvuru kararı çağıranın SelfSnap'inden (`snap.includes(i)`; ad yayılımı ve sarkan referansla aynı
 * kaynak). Ad eşleşmesi kayıtsız FK'yi "kendine" yapmaz (önceden yazılmış hedefle aynı ada konan tablo gibi). Verilmezse
 * mevcut ad/şemaya bakılır.
 */
export function fkRefColumnSource(d: TableDef, c: ConstraintDef, selfRef?: boolean): "draft" | "lookup" | "none" {
  if (c.kind.type !== "foreign_key" || c.kind.refTable === "") return "none";
  const self = selfRef ?? (c.kind.refSchema === d.schema && c.kind.refTable === d.name);
  return self ? "draft" : "lookup";
}

/**
 * Hedef adı taslağın şema/adıyla aynı ama FK kendine-başvuru olarak kayıtlı değil (SelfSnap'te yok): arayüz, hedefin
 * bu adlı mevcut (sunucudaki) tablo olduğunu ve kendine-başvuru için hedefin yeniden yazılması gerektiğini söyler.
 */
export function fkSelfNameOnly(d: TableDef, c: ConstraintDef, selfRef: boolean): boolean {
  return !selfRef && c.kind.type === "foreign_key" && c.kind.refTable !== "" && c.kind.refSchema === d.schema && c.kind.refTable === d.name;
}

/** Taslağın boş olmayan kolon adları (yinelenmeden, taslak sırasıyla); `current` listede yoksa başa eklenir. */
export function columnChoices(d: TableDef, current: string): string[] {
  const names = [...new Set(d.columns.map((c) => c.name).filter((n) => n !== ""))];
  return names.includes(current) ? names : [current, ...names];
}

/** Özgün kısıtın taslağa eşlenmiş yapısı (ad/notValid hariç) taslaktakinden farklı mı. */
function shapeChanged(original: TableDef, d: TableDef, names: Map<string, string>, o: ConstraintDef, c: ConstraintDef): boolean {
  const map = (a: readonly string[]) => a.map((n) => names.get(n) ?? `\u0000${n}`);
  const k = o.kind;
  let kind: ConstraintKind = k;
  if (k.type === "primary_key" || k.type === "unique") kind = { ...k, columns: map(k.columns), include: map(k.include) };
  else if (k.type === "foreign_key") {
    kind = isSelfIn(original, k)
      ? { ...k, columns: map(k.columns), refSchema: d.schema, refTable: d.name, refColumns: map(k.refColumns) }
      : { ...k, columns: map(k.columns) };
  }
  const shape = (x: ConstraintDef, kd: ConstraintKind) => JSON.stringify([kd, x.deferrable, x.initiallyDeferred]);
  return shape(o, kind) !== shape(c, c.kind);
}

const colSet = (a: readonly string[]) => JSON.stringify([...new Set(a)].sort());

/**
 * Yüklü kısıt yapısal olarak değişti mi / yeniden kurulacak mı (ad ve `notValid` hariç; arka ucun `same_structure` +
 * `diff` kurallarının istemci yaklaşımı). Özgün kolon adları `attnum` ile taslaktaki adlara eşlenir (kolon yeniden
 * adlandırma yapısal değişim sayılmaz); kendine-başvuran FK'nin hedef şema/ad/kolonları da taslağa eşlenir. Yapısı
 * aynı kalan kendine-başvuran FK, hedef kolon kümesi silinen ya da yeniden kurulan bir PK/UNIQUE'in kolon kümesiyse
 * arka uçta yeniden kurulur: o da değişmiş sayılır. Yeni kısıt / özgünü yok → false.
 */
export function constraintStructureChanged(original: TableDef | null, d: TableDef, c: ConstraintDef): boolean {
  const o = originalConstraint(original, c);
  if (!original || !o) return false;
  const names = currentColumnNames(original, d);
  if (shapeChanged(original, d, names, o, c)) return true;
  if (o.kind.type !== "foreign_key" || !isSelfIn(original, o.kind)) return false;
  const target = colSet(o.kind.refColumns);
  return original.constraints.some((ok) => {
    if (ok.oid === null || (ok.kind.type !== "primary_key" && ok.kind.type !== "unique") || colSet(ok.kind.columns) !== target) return false;
    const now = d.constraints.find((x) => x.oid === ok.oid);
    return !now || shapeChanged(original, d, names, ok, now);
  });
}

/** Bölümlenmiş tablo oluştururken CHECK NO INHERIT reddedilir: kutu kapalı (işaretliyse kaldırılabilsin diye açık kalır). */
export function noInheritBlocked(mode: "create" | "alter", d: TableDef, c: ConstraintDef): boolean {
  return mode === "create" && d.partitionBy !== null && c.kind.type === "check" && !c.kind.noInherit;
}

/** "Tabloyu tarar" notu: alter'da yeni kısıt; NOT VALID işaretli FK/CHECK taramaz (not çelişmesin diye gizlenir). */
export function scanNoteVisible(mode: "create" | "alter", c: ConstraintDef): boolean {
  return mode === "alter" && c.oid === null && !(supportsNotValid(c.kind.type) && c.notValid);
}

/** Ekle çubuğunun tür seçimi: PRIMARY KEY eklenemiyorsa UNIQUE'e kalıcı olarak geçer (PK silinince geri sıçramaz). */
export function stableAddType(choice: ConstraintType, pkAllowed: boolean): ConstraintType {
  return choice === "primary_key" && !pkAllowed ? "unique" : choice;
}

/**
 * Formun tek SelfSnap durumu. Taslak formun kendi düzenlemeleri dışında değişirse (ilk yükleme, yeniden yükleme,
 * uygulama sonrası yükleme) anlık görüntü o taslaktan `selfRefs` ile yeniden alınır ("düzenleme başlangıcı"); formun
 * düzenlemeleri `commit` ile yeni taslağı ve anlık görüntüyü birlikte kaydeder. Sinyal değildir: yalnızca düzenleme
 * anında okunur (görüntülenen bir değer değildir).
 */
export function createSnapTracker(read: () => TableDef | null) {
  let snap: SelfSnap = [];
  let own: TableDef | null = null;
  let seen: TableDef | null | undefined;
  const sync = () => {
    const d = read();
    if (d === seen) return;
    seen = d;
    if (d && d !== own) snap = selfRefs(d);
  };
  return {
    snap(): SelfSnap {
      sync();
      return snap;
    },
    /** Formun düzenlemesinden sonra: yeni taslak (denetleyicideki nesne) ve anlık görüntü. */
    commit(next: SelfSnap, draft: TableDef | null) {
      snap = next;
      own = draft;
      seen = draft;
    },
    /** Bu taslak formun son düzenlemesinin sonucu mu (değilse yükleme/yeniden yükleme ile değiştirilmiştir). */
    isOwn(draft: TableDef | null): boolean {
      return draft !== null && draft === own;
    },
  };
}
