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
  return { ...d, constraints: d.constraints.map((c, k) => (k === i ? f(c) : c)) };
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

export function canAddPrimaryKey(d: TableDef): boolean {
  return !d.constraints.some((c) => c.kind.type === "primary_key");
}

/** DEFERRABLE kapatılınca initiallyDeferred da false. */
export function withDeferrable(c: ConstraintDef, v: boolean): ConstraintDef {
  return { ...c, deferrable: v, initiallyDeferred: v ? c.initiallyDeferred : false };
}

/** deferrable false iken etkisiz (c aynen). */
export function withInitiallyDeferred(c: ConstraintDef, v: boolean): ConstraintDef {
  return c.deferrable ? { ...c, initiallyDeferred: v } : c;
}

/**
 * NOT VALID denetimi: "add" → yalnızca alter'da, yeni (oid null) FK/CHECK; "validate" → yüklü ve özgün tanımda
 * notValid=true olan kısıt ("Doğrula" kutusu notValid'i false yapar); "none" → gösterilmez.
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

/** Yüklü kısıtı (özgün kopyası) özgün sıradaki yerine (önündeki yüklü kısıt sayısı) geri koyar. */
export function restoreConstraint(d: TableDef, original: TableDef, oid: number): TableDef {
  if (d.constraints.some((c) => c.oid === oid)) return d;
  const pos = original.constraints.findIndex((c) => c.oid === oid);
  if (pos < 0) return d;
  const before = new Set(original.constraints.slice(0, pos).map((c) => c.oid));
  const at = d.constraints.filter((c) => c.oid !== null && before.has(c.oid)).length;
  const constraints = [...d.constraints];
  constraints.splice(at, 0, structuredClone(original.constraints[pos]!));
  return { ...d, constraints };
}

// ---- PRIMARY KEY üyelerinin NOT NULL'ı

/**
 * PRIMARY KEY üyesi kolonları notNull=true yapar (arka uç aksi halde hata verir). Yalnızca değişen kolonlar yeni nesne
 * olur; değişen yoksa aynı nesne döner. Ad eşleşmesi çağrı anındaki adlarla yapılır (Task 5/6 ilgili düzenleme
 * sonrasında çağırır).
 */
export function withPrimaryKeyNotNull(d: TableDef): TableDef {
  const members = new Set<string>();
  for (const c of d.constraints) if (c.kind.type === "primary_key") for (const n of c.kind.columns) if (n !== "") members.add(n);
  if (!d.columns.some((col) => !col.notNull && members.has(col.name))) return d;
  return { ...d, columns: d.columns.map((col) => (!col.notNull && members.has(col.name) ? { ...col, notNull: true } : col)) };
}

/** PRIMARY KEY üyesi olup notNull olmayan kolon adları (arayüz ipucu için). */
export function primaryKeyNullable(d: TableDef): string[] {
  const fixed = withPrimaryKeyNotNull(d);
  return fixed === d ? [] : d.columns.filter((col, i) => !col.notNull && fixed.columns[i]!.notNull).map((col) => col.name);
}

// ---- kolon listeleri (PK/UNIQUE columns + include)

export function constraintColumns(c: ConstraintDef, list: ColumnListName): string[] {
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

/** Boş/yinelenen ad eklenmez (aynı nesne). */
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
  return withFk(c, (k) => ({
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
