// Table taslağının saf modeli: kolon işlemleri, değer kaynağı, tip ön uyarısı, yeniden adlandırma yayılımı, bölümleme ve
// odak eşleme. İstemci SQL üretmez; hepsi değişmezdir (girdi nesnesi değiştirilmez, sınır dışı indeks → aynı nesne).
import { ttbl } from "../../app/strings-table";
import type { ColumnDef } from "../../ipc/bindings/ColumnDef";
import type { ConstraintKind } from "../../ipc/bindings/ConstraintKind";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PartitionKey } from "../../ipc/bindings/PartitionKey";
import type { PartitionStrategy } from "../../ipc/bindings/PartitionStrategy";
import type { TableDef } from "../../ipc/bindings/TableDef";
import { issuesFor, parseFocusField, type FieldIssues } from "./designer-model";
import { normalizeEmpty } from "./forms-model";

export type TableDraft = TableDef & { kind: "table" };

const inRange = (i: number, len: number) => Number.isInteger(i) && i >= 0 && i < len;

export function asTable(d: ObjectDef | null): TableDraft | null {
  return d && d.kind === "table" ? d : null;
}

export function tableEditPatch(d: ObjectDef, patch: Partial<TableDef>): ObjectDef {
  return d.kind === "table" ? { ...d, ...patch } : d;
}

/** Kolon/kısıt yapısı salt okunur mu (yüklenen tanıma göre): bölüm ya da kalıtımlı. */
export function structureLocked(original: TableDef | null): boolean {
  return !!original && (original.isPartition || original.inherits);
}

// ---- kolonlar

export function newColumn(): ColumnDef {
  return { attnum: null, name: "", dataType: "", notNull: false, default: null, identity: null, generated: null, collation: null, comment: null, using: null };
}

export function addTableColumn(d: TableDef): TableDef {
  return { ...d, columns: [...d.columns, newColumn()] };
}

export function updateTableColumn(d: TableDef, i: number, f: (c: ColumnDef) => ColumnDef): TableDef {
  if (!inRange(i, d.columns.length)) return d;
  return { ...d, columns: d.columns.map((c, k) => (k === i ? f(c) : c)) };
}

export function removeTableColumn(d: TableDef, i: number): TableDef {
  if (!inRange(i, d.columns.length)) return d;
  return { ...d, columns: d.columns.filter((_, k) => k !== i) };
}

/** Yüklenen tanımdaki eşi (attnum ile); yeni kolon → null. */
export function originalColumn(original: TableDef | null, c: ColumnDef): ColumnDef | null {
  if (!original || c.attnum === null) return null;
  return original.columns.find((o) => o.attnum === c.attnum) ?? null;
}

/** Taslakta olmayan yüklü kolonlar (özgün sırayla). */
export function removedColumns(original: TableDef | null, d: TableDef): ColumnDef[] {
  if (!original) return [];
  return original.columns.filter((o) => o.attnum !== null && !d.columns.some((c) => c.attnum === o.attnum));
}

/** Yüklü kolonu (özgün nesnenin kopyası) attnum sırasındaki yerine geri koyar; zaten varsa ya da bulunmazsa d. */
export function restoreColumn(d: TableDef, original: TableDef, attnum: number): TableDef {
  if (d.columns.some((c) => c.attnum === attnum)) return d;
  const src = original.columns.find((c) => c.attnum === attnum);
  if (!src) return d;
  // Yüklü kolonlar attnum sırasındadır ve yeni kolonlar her zaman sondadır: hedef yer, kendisinden küçük attnum'lu yüklü
  // kolonların sayısıdır (sıra değişmez → arka uçta "kolon sırası" uyarısı oluşmaz).
  const at = d.columns.filter((c) => c.attnum !== null && c.attnum < attnum).length;
  const columns = [...d.columns];
  columns.splice(at, 0, structuredClone(src));
  return { ...d, columns };
}

// ---- değer kaynağı (DEFAULT / IDENTITY / GENERATED birbirini dışlar)

export type ValueSource = "none" | "default" | "identity_always" | "identity_by_default" | "generated_stored" | "generated_virtual";

export function valueSourceOf(c: ColumnDef): ValueSource {
  if (c.generated) return c.generated.stored ? "generated_stored" : "generated_virtual";
  if (c.identity) return c.identity === "always" ? "identity_always" : "identity_by_default";
  return c.default !== null ? "default" : "none";
}

/** Kaynağın ifade metni (default ya da generated.expr; yoksa ""). */
export function valueExpr(c: ColumnDef): string {
  return c.generated ? c.generated.expr : c.default ?? "";
}

/**
 * İfade metni taşınır (default ↔ generated); identity seçilince notNull = true; diğer kaynaklar temizlenir.
 * `original` (yüklü eş) verilirse: yüklü kolonun kendi kaynağına dönüş default/identity/generated'ı birebir geri koyar; kimlik
 * seçiminin zorladığı notNull'dan çıkılınca (yüklü kolon kimlikli değilse) notNull özgün değerine döner.
 */
export function withValueSource(c: ColumnDef, s: ValueSource, original: ColumnDef | null = null): ColumnDef {
  const text = valueExpr(c);
  const leavingIdentity = c.identity !== null && s !== "identity_always" && s !== "identity_by_default";
  const restoreNotNull = !!original && leavingIdentity && original.identity === null ? original.notNull : null;
  if (original && s === valueSourceOf(original)) {
    return {
      ...c, default: original.default, identity: original.identity, generated: original.generated ? { ...original.generated } : null,
      notNull: leavingIdentity || original.identity !== null ? original.notNull : c.notNull,
    };
  }
  const base: ColumnDef = { ...c, default: null, identity: null, generated: null, notNull: restoreNotNull ?? c.notNull };
  switch (s) {
    case "none": return base;
    case "default": return { ...base, default: text };
    case "identity_always": return { ...base, identity: "always", notNull: true };
    case "identity_by_default": return { ...base, identity: "by_default", notNull: true };
    case "generated_stored": return { ...base, generated: { expr: text, stored: true } };
    case "generated_virtual": return { ...base, generated: { expr: text, stored: false } };
  }
}

/** Kaynağa göre default/generated.expr'e yazar (boş metin "" olarak kalır: alan hatası arka uçtan). */
export function withValueExpr(c: ColumnDef, text: string): ColumnDef {
  if (c.generated) return { ...c, generated: { ...c.generated, expr: text } };
  if (c.default !== null) return { ...c, default: text };
  return c;
}

/**
 * Satırda sunulan kaynaklar: yeni kolon → hepsi; yüklü ve üretilmiş olmayan → none/default/identity_*;
 * yüklü üretilmiş → none (DROP EXPRESSION) + kendi türü (stored/virtual).
 */
export function allowedValueSources(original: ColumnDef | null): ValueSource[] {
  if (!original) return ["none", "default", "identity_always", "identity_by_default", "generated_stored", "generated_virtual"];
  if (original.generated) return ["none", original.generated.stored ? "generated_stored" : "generated_virtual"];
  return ["none", "default", "identity_always", "identity_by_default"];
}

// ---- tip

/**
 * Tip metni karşılaştırma biçimi: kırp, boşlukları sadeleştir, tırnaksız kısımlarda büyük/küçük harf duyarsız ve noktalama
 * çevresindeki boşluk yok sayılır. Takma ad denkliği (int4 ↔ integer) arka uca bırakılır.
 */
function typeKey(t: string): string {
  return t.trim().split(/("(?:[^"]|"")*")/).map((part, i) => (i % 2 === 1 ? part : part.toLowerCase().replace(/\s+/g, " ").replace(/ ?([(),[\]]) ?/g, "$1"))).join("");
}
const sameTypeAndCollation = (c: ColumnDef, o: ColumnDef) => typeKey(c.dataType) === typeKey(o.dataType) && c.collation === o.collation;

function usingFor(c: ColumnDef, original: ColumnDef | null): ColumnDef {
  if (original && sameTypeAndCollation(c, original)) return { ...c, using: null };
  return c;
}

/** Tip/collation değişimi: değer özgün tipe ve collation'a dönerse `using` null'a çekilir. */
export function withDataType(c: ColumnDef, original: ColumnDef | null, dataType: string): ColumnDef {
  return usingFor({ ...c, dataType }, original);
}

export function withCollation(c: ColumnDef, original: ColumnDef | null, text: string): ColumnDef {
  return usingFor({ ...c, collation: normalizeEmpty(text) }, original);
}

export function usingVisible(original: ColumnDef | null, c: ColumnDef): boolean {
  return !!original && !sameTypeAndCollation(c, original);
}

/** serial ailesi → { dataType } (büyük/küçük harf duyarsız, isteğe bağlı `pg_catalog.`). */
export function serialFix(text: string): { dataType: string } | null {
  const m = /^(?:pg_catalog\.)?(serial|serial4|bigserial|serial8|smallserial|serial2)$/i.exec(text);
  if (!m) return null;
  switch (m[1]!.toLowerCase()) {
    case "serial": case "serial4": return { dataType: "integer" };
    case "bigserial": case "serial8": return { dataType: "bigint" };
    default: return { dataType: "smallint" };
  }
}

/**
 * İstemci tarafı ÖN uyarı (hata değil): ilk sorunun metni ya da null. Asıl dilbilgisi arka uçtadır (bilerek eksik:
 * yerleşik çok sözcüklü kalıplar ve 63 bayt sınırı).
 */
export function typeTextWarning(text: string): string | null {
  const w = ttbl.type;
  if (text === "") return w.required;
  if ([...text].length > 128) return w.tooLong;
  if (/[\u0000-\u001f\u007f]/.test(text)) return w.control;
  // Çift boşluk: tırnaklı kısımlar ve parantez içi hariç (sunucu bunları kabul eder).
  const spaced = text.replace(/"(?:[^"]|"")*"/g, "q").replace(/\([^)]*\)/g, "()");
  if (text !== text.trim() || spaced.includes("  ")) return w.spaces;
  if (serialFix(text) !== null) return w.serial;
  const rest = text.replace(/"(?:[^"]|"")*"/g, "");
  if (rest.includes("\"")) return w.unclosedQuote;
  if (rest.includes("--") || rest.includes("/*")) return w.comment;
  const bad = /[^A-Za-z0-9_$ .,()[\]-]/.exec(rest);
  if (bad) return w.forbidden(bad[0]);
  const opens = (rest.match(/\(/g) ?? []).length;
  const closes = (rest.match(/\)/g) ?? []).length;
  if (opens > 1 || closes !== opens) return w.modifier;
  if (opens === 1) {
    const a = rest.indexOf("(");
    const b = rest.indexOf(")");
    if (b < a || !/^\s*\d+\s*(,\s*-?\d+\s*)?$/.test(rest.slice(a + 1, b))) return w.modifier;
  }
  if (/[[\]]/.test(rest)) {
    const m = /(\[\d*\])+$/.exec(rest);
    if (!m || /[[\]]/.test(rest.slice(0, m.index))) return w.array;
  }
  return null;
}

/**
 * Satırdaki tip ön uyarısı: yüklü kolonun tipi özgün değerindeyse (karşılaştırma biçimiyle) uyarı yok — dokunulmamış
 * yüklü tipler (ör. `geometry(Point,4326)`, `"MyType"`) istemci dilbilgisine uymasa da sunucuda geçerlidir.
 */
export function columnTypeWarning(c: ColumnDef, original: ColumnDef | null): string | null {
  if (original && typeKey(c.dataType) === typeKey(original.dataType)) return null;
  return typeTextWarning(c.dataType);
}

/** serial hızlı düzeltmesi (tek düzenleme): tip → integer/bigint/smallint + GENERATED BY DEFAULT AS IDENTITY (notNull). */
export function applySerialFix(c: ColumnDef, original: ColumnDef | null): ColumnDef {
  const fix = serialFix(c.dataType);
  if (!fix) return c;
  return withValueSource(withDataType(c, original, fix.dataType), "identity_by_default", original);
}

/** Kolon adı bir PRIMARY KEY üyesi mi (NOT NULL kutusu kilitlenir; boş ad → false). */
export function primaryKeyMember(d: TableDef, name: string): boolean {
  return name !== "" && d.constraints.some((c) => c.kind.type === "primary_key" && c.kind.columns.includes(name));
}

/** Ayrıntılar alanındaki (collation/yorum/USING) hata: satırın ayrıntıları otomatik açılır. */
export function columnDetailsHaveIssues(issues: FieldIssues, i: number): boolean {
  return (["collation", "comment", "using"] as const).some((f) => issuesFor(issues, `columns[${i}].${f}`).length > 0);
}

/** Tip lookup sorgusu: kırpılmış metnin ilk `(`/`[` öncesi kısmı; boş → null (ilk 200 öneri). */
export function typeQueryOf(text: string): string | null {
  const t = text.trim();
  const cut = t.search(/[([]/);
  const q = (cut < 0 ? t : t.slice(0, cut)).trim();
  return q === "" ? null : q;
}

// ---- yeniden adlandırma yayılımı (konum anlık görüntüsü)

export type ColumnRef =
  | { at: "constraint"; c: number; list: "columns" | "include" | "refColumns"; j: number }
  | { at: "partitionKey"; j: number };

const targetsSelf = (d: TableDef, k: ConstraintKind) => k.type === "foreign_key" && k.refSchema === d.schema && k.refTable === d.name;

/**
 * `name`e başvuran konumlar: PK/UNIQUE `columns`+`include`, FK `columns` (ve FK kendine-başvuru anlık görüntüsündeyse
 * `refColumns`), bölümleme kolon anahtarları. CHECK ifadeleri SQL'dir, dokunulmaz. Boş ad → [].
 * Kendine-başvuru ADLA belirlenmez (`selfSnap`, formun SelfSnap'i): tablo geçici olarak başka bir tablonun adını taşırken
 * o tabloya giden FK'nin hedef kolonları yeniden adlandırılmamalı.
 */
export function columnRefs(d: TableDef, name: string, selfSnap: readonly number[]): ColumnRef[] {
  const out: ColumnRef[] = [];
  if (name === "") return out;
  d.constraints.forEach((con, c) => {
    const k = con.kind;
    const lists: Array<"columns" | "include" | "refColumns"> = [];
    if (k.type === "primary_key" || k.type === "unique") lists.push("columns", "include");
    else if (k.type === "foreign_key") { lists.push("columns"); if (selfSnap.includes(c)) lists.push("refColumns"); }
    for (const list of lists) {
      const arr = (k as unknown as Record<string, string[]>)[list]!;
      arr.forEach((n, j) => { if (n === name) out.push({ at: "constraint", c, list, j }); });
    }
  });
  d.partitionBy?.keys.forEach((key, j) => { if (key.kind === "column" && key.name === name) out.push({ at: "partitionKey", j }); });
  return out;
}

/** i. kolonun adını `next` yapar ve YALNIZCA verilen konumları aynı değere yazar (konum artık o adı taşımasa bile). */
export function renameTableColumn(d: TableDef, i: number, next: string, refs: readonly ColumnRef[]): TableDef {
  if (!inRange(i, d.columns.length)) return d;
  const columns = d.columns.map((c, k) => (k === i ? { ...c, name: next } : c));
  const constraints = d.constraints.map((k, c) => {
    const mine = refs.filter((r): r is Extract<ColumnRef, { at: "constraint" }> => r.at === "constraint" && r.c === c);
    if (mine.length === 0) return k;
    const kind = structuredClone(k.kind) as Record<string, unknown>;
    for (const r of mine) {
      const list = kind[r.list];
      if (Array.isArray(list) && inRange(r.j, list.length)) list[r.j] = next;
    }
    return { ...k, kind: kind as unknown as ConstraintKind };
  });
  const keys = d.partitionBy?.keys.map((key, j) =>
    refs.some((r) => r.at === "partitionKey" && r.j === j) && key.kind === "column" ? { ...key, name: next } : key);
  return { ...d, columns, constraints, partitionBy: d.partitionBy && keys ? { ...d.partitionBy, keys } : d.partitionBy };
}

/** Tabloyu hedefleyen FK'lerin konumları (refSchema/refTable = taslağın şema/adı). */
export function selfRefs(d: TableDef): number[] {
  const out: number[] = [];
  d.constraints.forEach((con, c) => { if (targetsSelf(d, con.kind)) out.push(c); });
  return out;
}

/** Tablo adı/şeması değişimi; verilen kendine-başvuran FK'lerin refSchema/refTable'ı da yeni değere yazılır. */
export function renameTable(d: TableDef, patch: { name?: string; schema?: string }, selfRefIdx: readonly number[]): TableDef {
  const next: TableDef = { ...d, ...(patch.name !== undefined ? { name: patch.name } : {}), ...(patch.schema !== undefined ? { schema: patch.schema } : {}) };
  const constraints = d.constraints.map((con, c) => {
    if (!selfRefIdx.includes(c) || con.kind.type !== "foreign_key") return con;
    return { ...con, kind: { ...con.kind, refSchema: next.schema, refTable: next.name } };
  });
  return { ...next, constraints };
}

// ---- bölümleme (yalnız create)

export function withPartitionStrategy(d: TableDef, s: PartitionStrategy | null): TableDef {
  if (s === null) return { ...d, partitionBy: null };
  return { ...d, partitionBy: d.partitionBy ? { ...d.partitionBy, strategy: s } : { strategy: s, keys: [] } };
}

export function addPartitionKey(d: TableDef, kind: PartitionKey["kind"]): TableDef {
  if (!d.partitionBy) return d;
  const key: PartitionKey = kind === "column" ? { kind: "column", name: "" } : { kind: "expression", sql: "" };
  return { ...d, partitionBy: { ...d.partitionBy, keys: [...d.partitionBy.keys, key] } };
}

export function updatePartitionKey(d: TableDef, j: number, key: PartitionKey): TableDef {
  if (!d.partitionBy || !inRange(j, d.partitionBy.keys.length)) return d;
  return { ...d, partitionBy: { ...d.partitionBy, keys: d.partitionBy.keys.map((k, n) => (n === j ? key : k)) } };
}

export function removePartitionKey(d: TableDef, j: number): TableDef {
  if (!d.partitionBy || !inRange(j, d.partitionBy.keys.length)) return d;
  return { ...d, partitionBy: { ...d.partitionBy, keys: d.partitionBy.keys.filter((_, n) => n !== j) } };
}

/** Anahtar türünü değiştirir; metin taşınır (ad ↔ sql). */
export function switchPartitionKeyKind(k: PartitionKey, kind: PartitionKey["kind"]): PartitionKey {
  if (k.kind === kind) return k;
  return k.kind === "column" ? { kind: "expression", sql: k.name } : { kind: "column", name: k.sql };
}

/** Anahtarın kısa etiketi (sil düğmesinin erişilebilir adı): kolon adı ya da ifade (uzunsa kısaltılır); boş → "boş". */
export function partitionKeyLabel(k: PartitionKey): string {
  const text = (k.kind === "column" ? k.name : k.sql).replace(/\s+/g, " ").trim();
  if (text === "") return ttbl.partitioning.emptyKey;
  return text.length > 40 ? `${text.slice(0, 39)}…` : text;
}

/** Kolon anahtarı seçenekleri: taslak kolon adları (boşlar hariç, yinelenmeden); seçili değer listede yoksa başa. */
export function partitionColumnChoices(d: TableDef, current: string): string[] {
  const names = [...new Set(d.columns.map((c) => c.name).filter((n) => n !== ""))];
  return names.includes(current) ? names : [current, ...names];
}

// ---- odak

/** Odak alanından bölüm + satır: column → columns ve attnum'u eşleşen satır; constraint → oid; bulunamazsa index null. */
export function rowOfFocus(d: TableDef, field: string): { section: "columns" | "constraints"; index: number | null } | null {
  const f = parseFocusField(field);
  if (!f) return null;
  if (f.kind === "column") {
    const i = d.columns.findIndex((c) => c.attnum === f.id);
    return { section: "columns", index: i < 0 ? null : i };
  }
  const i = d.constraints.findIndex((c) => c.oid === f.id);
  return { section: "constraints", index: i < 0 ? null : i };
}

// ---- genel

export type TableSection = "general" | "columns" | "constraints" | "partitioning";

/** Görünen bölümler: create'te partitioning dahil; alter'da yok. */
export function tableSections(mode: "create" | "alter"): TableSection[] {
  return mode === "create" ? ["general", "columns", "constraints", "partitioning"] : ["general", "columns", "constraints"];
}

/**
 * Bölüm sekmesinde hata rozeti: `columns[…`, `constraints`/`constraints[…`, `partitionBy…` yolları; general → öteki
 * yollar ya da genel şerit (üst-düzey `columns` hatası genel şeride düşer).
 */
export function sectionHasIssues(issues: FieldIssues, section: TableSection): boolean {
  const sec = (p: string): TableSection =>
    p.startsWith("columns[") ? "columns" : p === "constraints" || p.startsWith("constraints[") ? "constraints" : p.startsWith("partitionBy") ? "partitioning" : "general";
  if (section === "general" && issues.general.length > 0) return true;
  return [...issues.byPath.keys()].some((p) => sec(p) === section);
}

/**
 * Bölümlenmiş tablonun depolama kuralları (arka uç: bölümlenmiş tablonun kendi depolaması yoktur). Her zaman reddedilen
 * denetimler kilitlenir — değer varsayılan/yüklenen değerdeyken (aksi halde temizlenebilsin diye açık kalır); sürüme bağlı
 * kurallar yalnızca ipucudur (arayüz sunucu sürümünü bilmez).
 * - alter, yüklenen tablo bölümlenmiş (`partitionText`): kalıcılık ve fillfactor değişimi her zaman hata; tablespace PG 12+.
 * - create, `partitionBy` dolu: fillfactor her zaman hata; `pg_default` açıkça belirtilemez (PG 12+; PG < 12 hiç
 *   tablespace yok); UNLOGGED PG 18+ hata (yalnızca ipucu).
 */
export interface StorageGate {
  persistenceLocked: boolean;
  fillfactorLocked: boolean;
  /** `pg_default` seçeneği kapalı (seçili değilse). */
  pgDefaultBlocked: boolean;
  persistenceHint: string | null;
  fillfactorHint: string | null;
  tablespaceHint: string | null;
}

export function storageGate(mode: "create" | "alter", original: TableDef | null, d: TableDef): StorageGate {
  const g = ttbl.general;
  const loaded = mode === "alter" && original !== null && original.partitionText !== null ? original : null;
  const createP = mode === "create" && d.partitionBy !== null;
  return {
    persistenceLocked: !!loaded && d.persistence === loaded.persistence,
    fillfactorLocked: (!!loaded && d.fillfactor === loaded.fillfactor) || (createP && d.fillfactor === null),
    pgDefaultBlocked: createP,
    persistenceHint: loaded ? g.partitionedPersistence : createP ? g.partitionedUnlogged : null,
    fillfactorHint: loaded || createP ? g.partitionedFillfactor : null,
    tablespaceHint: loaded ? g.partitionedTablespaceAlter : createP ? g.partitionedTablespaceCreate : null,
  };
}
