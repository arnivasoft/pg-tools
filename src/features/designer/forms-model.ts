// Designer formlarının saf yardımcıları (tembel parça): alan yolu sabitleri, boş → null normalleştirme, taslak alan
// güncelleyicileri, eklenti listesi/sürüm notu, sequence yön/biçim yardımcıları ve indeks form kuralları. SQL üretmez;
// adlar yazıldığı gibi gider (yalnızca boş → null).
import { tdes } from "../../app/strings-designer";
import type { ExtensionDef } from "../../ipc/bindings/ExtensionDef";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { IndexKey } from "../../ipc/bindings/IndexKey";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { SchemaDef } from "../../ipc/bindings/SchemaDef";
import type { SeqType } from "../../ipc/bindings/SeqType";
import type { SequenceDef } from "../../ipc/bindings/SequenceDef";
import { isSafeIntegerString, type FieldIssues } from "./designer-model";

/** Formun bildiği `FieldError.path` kökleri (dışındakiler genel hata şeridine düşer). */
export const SCHEMA_PATHS: ReadonlySet<string> = new Set(["name", "owner", "comment"]);
export const EXTENSION_PATHS: ReadonlySet<string> = new Set(["name", "schema", "version"]);
/** `ownedBy` kökü `ownedBy.schema|table|column` alt yollarını da kapsar (`groupIssues` kök/alt yol kuralı). */
export const SEQUENCE_PATHS: ReadonlySet<string> = new Set([
  "name", "schema", "owner", "dataType", "increment", "minValue", "maxValue", "start", "cache", "cycle", "ownedBy", "comment",
]);
/**
 * `columns` kökü `columns[i]` ve `columns[i].key|opclass|collation`, `include` kökü `include[i]` yollarını kapsar
 * (`groupIssues` kök/alt yol kuralı; `columns[1]` ⊅ `columns[10]`). Üst-düzey `columns` hatası (boş liste, son-ifade
 * taraması) yine genel şeride düşer. İndekste `schema`/`table` salt okunurdur: hataları genel şeritte görünür.
 */
export const INDEX_PATHS: ReadonlySet<string> = new Set([
  "name", "method", "unique", "columns", "include", "tablespace", "fillfactor", "whereClause", "comment", "constraint",
]);
/** Tür → bilinen yollar (`FORMS` tablosu bunu kullanır; her `ObjectKind` için bir form olmalı — testte denetlenir). */
export const FORM_PATHS: Record<ObjectKind, ReadonlySet<string>> = {
  schema: SCHEMA_PATHS,
  extension: EXTENSION_PATHS,
  sequence: SEQUENCE_PATHS,
  index: INDEX_PATHS,
};

/**
 * İsteğe bağlı alan: boş ya da yalnızca boşluk → `null`; aksi halde metin OLDUĞU GİBİ (kırpılmaz — kullanıcının yazdığı
 * gönderilir, geçersizse arka uç alan hatası döner).
 */
export function normalizeEmpty(s: string): string | null {
  return s.trim() === "" ? null : s;
}

/** Taslak şema mı (aksi halde null; form yalnızca kendi türünü düzenler). */
export function asSchema(d: ObjectDef | null): (SchemaDef & { kind: "schema" }) | null {
  return d && d.kind === "schema" ? d : null;
}
export function asExtension(d: ObjectDef | null): (ExtensionDef & { kind: "extension" }) | null {
  return d && d.kind === "extension" ? d : null;
}

/** Şema taslağına alan güncellemesi uygular (yeni tanım döner; tür uyuşmazsa tanım değişmez). */
export function schemaEditPatch(d: ObjectDef, patch: Partial<SchemaDef>): ObjectDef {
  return d.kind === "schema" ? { ...d, ...patch } : d;
}

/** Eklenti taslağına alan güncellemesi uygular (yeni tanım döner; tür uyuşmazsa tanım değişmez). */
export function extensionEditPatch(d: ObjectDef, patch: Partial<ExtensionDef>): ObjectDef {
  return d.kind === "extension" ? { ...d, ...patch } : d;
}

/** `datalist` seçeneğinin yan metni: arka ucun `detail` metni (yoksa değerin kendisi). */
export function extensionOptionLabel(item: LookupItem): string {
  const d = item.detail?.trim();
  return d ? d : item.value;
}

/** Yazılan ada karşılık gelen kurulabilir eklenti (tam eşleşme, büyük/küçük harf duyarlı). */
export function findExtension(items: readonly LookupItem[], name: string): LookupItem | null {
  return items.find((i) => i.value === name) ?? null;
}

/**
 * Alter'da seçilen sürüm yüklenenden farklıysa `UPDATE TO` çalışır → not gösterilir. Boş sürüm (`null`) = değişmez;
 * create'te (yüklenen tanım yok) not yok.
 */
export function extensionVersionNote(original: ObjectDef | null, draft: ExtensionDef): boolean {
  if (!original || original.kind !== "extension") return false;
  return draft.version !== null && draft.version !== original.version;
}

/** Yalnızca tam bu yoldaki hatalar (alt yollar hariç; ör. satır düzeyi `columns[1]` hatası, alanları `columns[1].key`). */
export function exactIssues(issues: FieldIssues, path: string): string[] {
  return issues.byPath.get(path) ?? [];
}

/**
 * Listede artık olmayan satırlara (`root[i]…`, i ≥ len; ör. satır silindikten sonra gelen bayat önizleme) ait hatalar —
 * kaybolmasınlar diye liste düzeyinde gösterilir.
 */
export function orphanRowIssues(issues: FieldIssues, root: string, len: number): string[] {
  const out: string[] = [];
  const prefix = `${root}[`;
  for (const [p, msgs] of issues.byPath) {
    if (!p.startsWith(prefix)) continue;
    const close = p.indexOf("]", prefix.length);
    const idx = close < 0 ? "" : p.slice(prefix.length, close);
    if (/^[0-9]+$/.test(idx) && Number(idx) >= len) out.push(...msgs);
  }
  return out;
}

// ---- Sequence ----

export function asSequence(d: ObjectDef | null): (SequenceDef & { kind: "sequence" }) | null {
  return d && d.kind === "sequence" ? d : null;
}

/** Sequence taslağına alan güncellemesi uygular (yeni tanım döner; tür uyuşmazsa tanım değişmez). */
export function sequenceEditPatch(d: ObjectDef, patch: Partial<SequenceDef>): ObjectDef {
  return d.kind === "sequence" ? { ...d, ...patch } : d;
}

/** Artış metninin yönü: geçerli ve sıfırdan farklıysa 1/-1; aksi halde null (yazım sürüyor ya da geçersiz). */
export function incrementSign(s: string): 1 | -1 | null {
  if (!isSafeIntegerString(s)) return null;
  const n = BigInt(s);
  return n > 0n ? 1 : n < 0n ? -1 : null;
}

/**
 * Artış düzenlemesi + yön değişimi uyarlaması. Önceki yön `d.increment`'ten (geçersizse `fallbackSign`; ör. yazım
 * sırasındaki "-" ara durumu); yön değişirse (artan↔azalan) yalnızca BOŞ ya da önceki yönün VARSAYILANINDAKİ alanlar
 * yeni yönün varsayılanına çekilir — artan: min = null (ya da "1"), max = null, start = "1"; azalan: min = null,
 * max = "-1", start = "-1". Elle girilmiş başka değerler korunur (uyumsuzsa arka uç alan hatası gösterir). `dataType`
 * dokunulmaz.
 */
export function incrementEditPatch(d: SequenceDef, next: string, fallbackSign: 1 | -1 = 1): SequenceDef {
  const prev = incrementSign(d.increment) ?? fallbackSign;
  const sign = incrementSign(next);
  const out: SequenceDef = { ...d, increment: next };
  if (sign === null || sign === prev) return out;
  const blank = (v: string | null) => v === null || v.trim() === "";
  if (sign === -1) {
    if (blank(d.minValue) || d.minValue === "1") out.minValue = null;
    if (blank(d.maxValue)) out.maxValue = "-1";
    if (blank(d.start) || d.start === "1") out.start = "-1";
  } else {
    if (blank(d.minValue)) out.minValue = null;
    if (blank(d.maxValue) || d.maxValue === "-1") out.maxValue = null;
    if (blank(d.start) || d.start === "-1") out.start = "1";
  }
  return out;
}

/** Alter: artışın yönü yüklenen tanıma göre değişti mi ("sayaç sıfırlanır" notu). Geçersiz artış → false. */
export function sequenceDirectionChanged(original: ObjectDef | null, d: SequenceDef): boolean {
  if (!original || original.kind !== "sequence") return false;
  const a = incrementSign(original.increment);
  const b = incrementSign(d.increment);
  return a !== null && b !== null && a !== b;
}

/** Alter: başlangıç değeri yön değişmeden değişti mi ("mevcut sayacı değiştirmez" notu). */
export function sequenceStartChanged(original: ObjectDef | null, d: SequenceDef): boolean {
  if (!original || original.kind !== "sequence") return false;
  return d.start !== original.start && !sequenceDirectionChanged(original, d);
}

/** Alter: veri türü değişti mi. */
export function sequenceTypeChanged(original: ObjectDef | null, d: SequenceDef): boolean {
  return !!original && original.kind === "sequence" && original.dataType !== d.dataType;
}

const SEQ_BOUNDS: Record<SeqType, [string, string]> = {
  smallint: ["-32768", "32767"],
  integer: ["-2147483648", "2147483647"],
  bigint: ["-9223372036854775808", "9223372036854775807"],
};
/** Veri türünün açık sınırları (ipucu metni). */
export function seqTypeBoundsNote(t: SeqType): string {
  const [lo, hi] = SEQ_BOUNDS[t];
  return tdes.sequence.bounds(t, lo, hi);
}

export type SequenceNumberField = "increment" | "minValue" | "maxValue" | "start" | "cache";
/**
 * Sayı alanlarının yalnızca BİÇİM uyarısı (`isSafeIntegerString`): zorunlu alan boşsa "gerekli", yazılmış ama tam sayı
 * değilse "biçim". Aralık/sıra kuralları arka ucun `FieldError`'larıdır. Biçimsiz sayı JSON'da çözülemez (önizleme alan
 * hatası yerine önizleme hatası döner) — bu uyarı nedeni alanın yanında gösterir.
 */
export function sequenceFormatWarnings(d: SequenceDef): Partial<Record<SequenceNumberField, string>> {
  const out: Partial<Record<SequenceNumberField, string>> = {};
  for (const k of ["increment", "start", "cache"] as const) {
    const v = d[k];
    if (v.trim() === "") out[k] = tdes.sequence.required;
    else if (!isSafeIntegerString(v)) out[k] = tdes.sequence.format;
  }
  for (const k of ["minValue", "maxValue"] as const) {
    const v = d[k];
    if (v !== null && !isSafeIntegerString(v)) out[k] = tdes.sequence.format;
  }
  return out;
}

/** OWNED BY tablosu sequence ile aynı şemada değilse ipucu (arka uç da `ownedBy.schema` hatası verir). */
export function ownedByHint(d: SequenceDef): string | null {
  return d.ownedBy && d.ownedBy.schema !== d.schema ? tdes.sequence.ownedBySchemaMismatch : null;
}

// ---- Index ----

export function asIndex(d: ObjectDef | null): (IndexDef & { kind: "index" }) | null {
  return d && d.kind === "index" ? d : null;
}

/** İndeks taslağına alan güncellemesi uygular (yeni tanım döner; tür uyuşmazsa tanım değişmez). */
export function indexEditPatch(d: ObjectDef, patch: Partial<IndexDef>): ObjectDef {
  return d.kind === "index" ? { ...d, ...patch } : d;
}

/** ASC/DESC ve NULLS FIRST/LAST yalnızca btree'de geçerli. */
export function btreeOnlyFields(method: string): boolean {
  return method === "btree";
}

/** fillfactor: gin ve brin desteklemez (arka uç reddeder). */
export function fillfactorEnabled(method: string): boolean {
  return method !== "gin" && method !== "brin";
}

/**
 * Yöntem değişimi: btree dışı yöntemde devre dışı kalan DESC/NULLS seçimleri ve gin/brin'de fillfactor temizlenir
 * (aksi halde denetimi kapalı olduğu için kullanıcının düzeltemeyeceği bir arka uç hatası kalırdı).
 */
export function indexMethodPatch(d: IndexDef, method: string): IndexDef {
  const out: IndexDef = { ...d, method };
  if (!btreeOnlyFields(method) && d.columns.some((c) => c.descending || c.nullsFirst !== null)) {
    out.columns = d.columns.map((c) => ({ ...c, descending: false, nullsFirst: null }));
  }
  if (!fillfactorEnabled(method)) out.fillfactor = null;
  return out;
}

export type NullsSelect = "default" | "first" | "last";
export function nullsFirstToSelect(v: boolean | null): NullsSelect {
  return v === null ? "default" : v ? "first" : "last";
}
export function selectToNullsFirst(s: string): boolean | null {
  return s === "first" ? true : s === "last" ? false : null;
}

/** Satırın kısa etiketi (taşı/sil düğmelerinin erişilebilir adı için): sütun adı ya da ifade (uzunsa kısaltılır). */
export function columnKeyLabel(key: IndexKey): string {
  const text = (key.kind === "column" ? key.name : key.sql).replace(/\s+/g, " ").trim();
  if (text === "") return tdes.index.empty;
  return text.length > 40 ? `${text.slice(0, 39)}…` : text;
}

/** Anahtar türü değişimi: yazılmış metin taşınır (sütun adı ↔ ifade metni). */
export function switchKeyKind(key: IndexKey, kind: IndexKey["kind"]): IndexKey {
  if (key.kind === kind) return key;
  return key.kind === "expression" ? { kind: "column", name: key.sql } : { kind: "expression", sql: key.name };
}

export function newIndexColumn(): IndexColumn {
  return { key: { kind: "column", name: "" }, opclass: null, collation: null, descending: false, nullsFirst: null };
}

/** INCLUDE listesinde bir öğeyi değiştirir (değişmez; sınır dışı → aynı nesne). */
export function updateInclude(d: IndexDef, i: number, name: string): IndexDef {
  if (!Number.isInteger(i) || i < 0 || i >= d.include.length) return d;
  return { ...d, include: d.include.map((c, k) => (k === i ? name : c)) };
}

/**
 * Tablespace seçimi: temizleme yoktur. Boş seçim yalnızca yüklenen tanımda tablespace yoksa (`original === null`, create
 * dahil) anlamlıdır ve `null` kalır; yüklenen değer varken boş seçim yüklenen değeri korur (değişiklik gönderilmez).
 * Varsayılana dönmek için `pg_default` açıkça seçilir.
 */
export function normalizeTablespaceEdit(original: string | null, next: string): string | null {
  return normalizeEmpty(next) ?? original;
}

/** Sayı girdisinden fillfactor: boş → null; tam sayı değilse null (aralık 10–100 arka uçta denetlenir). */
export function fillfactorFromInput(s: string): number | null {
  const t = s.trim();
  return /^[0-9]+$/.test(t) ? Number(t) : null;
}

/**
 * Yüklenen tanıma göre şeritler: kısıta (PK/UNIQUE/EXCLUDE) ait indeks → form tümüyle salt okunur (metin);
 * `valid === false` → uyarı metni.
 */
export function indexReadOnlyReason(def: IndexDef | null): { constraint: string | null; invalid: string | null } {
  if (!def) return { constraint: null, invalid: null };
  return {
    constraint: def.constraint !== null ? tdes.index.constraint(def.constraint) : null,
    invalid: def.valid === false ? tdes.index.invalid : null,
  };
}

const sameKey = (a: IndexKey, b: IndexKey) =>
  a.kind === "column" ? b.kind === "column" && a.name === b.name : b.kind === "expression" && a.sql === b.sql;
const sameColumn = (a: IndexColumn, b: IndexColumn) =>
  sameKey(a.key, b.key) && a.opclass === b.opclass && a.collation === b.collation && a.descending === b.descending && a.nullsFirst === b.nullsFirst;

/**
 * Alter: yerinde değiştirilemeyen fark (→ DROP + CREATE). Arka ucun `structural_change` alan kümesiyle aynı: method,
 * unique, columns, include, whereClause (+ schema/table; formda salt okunurdur).
 */
export function structuralChange(original: IndexDef, draft: IndexDef): boolean {
  return (
    original.method !== draft.method ||
    original.unique !== draft.unique ||
    original.columns.length !== draft.columns.length ||
    original.columns.some((c, i) => !sameColumn(c, draft.columns[i]!)) ||
    original.include.length !== draft.include.length ||
    original.include.some((c, i) => c !== draft.include[i]) ||
    original.whereClause !== draft.whereClause ||
    original.table !== draft.table ||
    original.schema !== draft.schema
  );
}
