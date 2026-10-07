// Veri düzenleyici düzenleme katmanı (saf, durumsuz): yalnızca değişen hücreleri tutar, `DataChangeSet`
// değişikliklerini üretir ve başarılı uygulama sonuçlarını yüklenen satırlara işler. Solid/DOM yok; girdiler
// hiçbir zaman değiştirilmez (işlem yoksa aynı nesne döner). Değerler günlüğe yazılmaz.
// Arka uç kuralları `crates/pgt-ddl/src/data/plan.rs` (`column_error`) ile aynıdır.
import type { CellEdit } from "../../ipc/bindings/CellEdit";
import type { CellValue } from "../../ipc/bindings/CellValue";
import type { DataColumn } from "../../ipc/bindings/DataColumn";
import type { DataSource } from "../../ipc/bindings/DataSource";
import type { RowChange } from "../../ipc/bindings/RowChange";
import type { RowResult } from "../../ipc/bindings/RowResult";

/** Kolonlar kaynak sırasıyla + (hasVersion ise) son eleman sürüm (`xmin` metni). */
export type Row = (string | null)[];

export interface Loaded {
  source: DataSource;
  rows: Row[];
  versionIndex: number;
  hasVersion: boolean;
}

export interface NewRow {
  id: number;
  /** Yalnızca dokunulan kolonlar. */
  values: ReadonlyMap<string, CellValue>;
}

/** Değişiklik sırasındaki yer: var olan satır (Update ya da Delete) veya yeni satır (Insert). */
export type EditSlot = { kind: "row"; rowIndex: number } | { kind: "new"; newId: number };

/**
 * Düzenleme durumu. `cells`/`deleted`/`order` satır indeksleri yalnızca kurulduğu `Loaded` için geçerlidir:
 * denetleyici `Loaded` her değiştiğinde (yeniden yükleme, `applyResults`) durumu `EMPTY_EDIT`'e sıfırlamalıdır.
 */
export interface EditState {
  /** rowIndex → kolon → yeni değer; yalnızca özgünden farklı olanlar. */
  cells: ReadonlyMap<number, ReadonlyMap<string, CellValue>>;
  inserted: readonly NewRow[];
  /** Silinecek rowIndex'ler (düzenlemeleri `cells`'te tutulur ama gönderilmez). */
  deleted: ReadonlySet<number>;
  /** İlk dokunuş sırası: `toChangeSet` bu sırayı korur (ör. önce DELETE sonra aynı anahtarla INSERT). */
  order: readonly EditSlot[];
  /** Sonraki yeni satır kimliği (kaldırılan kimlik yeniden kullanılmaz). */
  nextId: number;
}

export type ChangeOrigin = { kind: "insert"; newId: number } | { kind: "update" | "delete"; rowIndex: number };

export type CellReason = "readOnly" | "generated" | "identityAlways" | "notUpdatable" | "notInsertable" | "deleted";
export interface CellState {
  editable: boolean;
  reason?: CellReason;
}

export const EMPTY_EDIT: EditState = Object.freeze({
  cells: new Map(),
  inserted: Object.freeze([]) as readonly NewRow[],
  deleted: new Set<number>(),
  order: Object.freeze([]) as readonly EditSlot[],
  nextId: 1,
});

// --- Yardımcılar ---------------------------------------------------------------------------------

/** Kaynak yazılabilir mi (savunma: anahtar ve sürüm de gerekir; view/MV `hasVersion=false`). */
function writable(l: Loaded): boolean {
  return l.source.editable && l.source.key !== null && l.hasVersion;
}

function colIndex(l: Loaded, column: string): number {
  return l.source.columns.findIndex((c) => c.name === column);
}

/** Özgün satırın anahtar değerleri (`DataKey.columns` sırasıyla) ve sürümü; eksik/null varsa `null`. */
function identity(l: Loaded, row: number): { key: string[]; version: string } | null {
  const r = l.rows[row];
  const key = l.source.key;
  if (!r || !key || !l.hasVersion) return null;
  const version = r[l.versionIndex];
  if (typeof version !== "string") return null;
  const values: string[] = [];
  for (const name of key.columns) {
    const i = colIndex(l, name);
    const v = i < 0 ? null : r[i];
    if (typeof v !== "string") return null;
    values.push(v);
  }
  return { key: values, version };
}

/** `Default` hiçbir özgün değere eşit değildir; `Null` yalnızca özgün `null`'a, `Text` aynı metne eşittir. */
function sameAsOriginal(orig: string | null | undefined, v: CellValue): boolean {
  if (v.kind === "default") return false;
  if (v.kind === "null") return orig === null;
  return orig === v.value;
}

function columnUpdateReason(c: DataColumn, s: DataSource): CellReason | undefined {
  if (c.generated) return "generated";
  if (c.identity === "always") return "identityAlways";
  if (!s.canUpdate || !c.updatable) return "notUpdatable";
  return undefined;
}

/** `insertable=false` ALWAYS kimlikten önce: yetkisiz kolona `Default` bile yazılamaz (arka uç `column_error`). */
function columnInsertReason(c: DataColumn, s: DataSource): CellReason | undefined {
  if (!s.canInsert) return "notInsertable";
  if (c.generated) return "generated";
  if (!c.insertable) return "notInsertable";
  if (c.identity === "always") return "identityAlways";
  return undefined;
}

function hasSlot(order: readonly EditSlot[], row: number): boolean {
  return order.some((s) => s.kind === "row" && s.rowIndex === row);
}

/** Satırın değişikliği varsa sırada yeri olmasını, yoksa olmamasını sağlar. */
function syncRowSlot(
  order: readonly EditSlot[],
  row: number,
  cells: ReadonlyMap<number, ReadonlyMap<string, CellValue>>,
  deleted: ReadonlySet<number>,
): readonly EditSlot[] {
  const changed = cells.has(row) || deleted.has(row);
  const present = hasSlot(order, row);
  if (changed && !present) return [...order, { kind: "row", rowIndex: row }];
  if (!changed && present) return order.filter((s) => !(s.kind === "row" && s.rowIndex === row));
  return order;
}

function withCell(e: EditState, row: number, column: string, value: CellValue | null): EditState {
  const prev = e.cells.get(row);
  if (value === null && !prev?.has(column)) return e;
  const rowMap = new Map(prev ?? []);
  if (value === null) rowMap.delete(column);
  else rowMap.set(column, value);
  const cells = new Map(e.cells);
  if (rowMap.size === 0) cells.delete(row);
  else cells.set(row, rowMap);
  return { ...e, cells, order: syncRowSlot(e.order, row, cells, e.deleted) };
}

// --- Durum sorguları -----------------------------------------------------------------------------

/** Gönderilecek değişiklik sayısı (= `toChangeSet(l, e).changes.length`; rozet her zaman yükle eşit). */
export function changeCount(l: Loaded, e: EditState): number {
  return toChangeSet(l, e).changes.length;
}

/**
 * Durum tabanlı ucuz denetim: değişmiş satır ya da dokunulmuş yeni satır var mı. `changeCount` 0 iken `true`
 * olabilmesi yalnızca bayat durumda (başka `Loaded`'a karşı kurulmuş ya da kaynak sonradan salt okunur) mümkündür.
 */
export function isDirty(e: EditState): boolean {
  return e.order.some((s) =>
    s.kind === "row" ? true : (e.inserted.find((r) => r.id === s.newId)?.values.size ?? 0) > 0,
  );
}

/** Hücre düzenlenebilir mi ve değilse neden. Yeni satırda `identityAlways` yalnızca `Default` kabul eder. */
export function cellState(l: Loaded, e: EditState, row: number | { newId: number }, column: string): CellState {
  const ro: CellState = { editable: false, reason: "readOnly" };
  if (!writable(l)) return ro;
  const c = l.source.columns.find((k) => k.name === column);
  if (!c) return ro;
  if (typeof row === "number") {
    if (!identity(l, row)) return ro;
    if (e.deleted.has(row)) return { editable: false, reason: "deleted" };
    const reason = columnUpdateReason(c, l.source);
    return reason ? { editable: false, reason } : { editable: true };
  }
  if (!e.inserted.some((r) => r.id === row.newId)) return ro;
  const reason = columnInsertReason(c, l.source);
  return reason ? { editable: false, reason } : { editable: true };
}

/** Görüntülenecek değer: düzenleme varsa o (`changed`), yoksa özgün değer. */
export function cellValueAt(
  l: Loaded,
  e: EditState,
  row: number,
  column: string,
): { value: CellValue | { kind: "orig"; value: string | null }; changed: boolean } {
  const edited = e.cells.get(row)?.get(column);
  if (edited) return { value: edited, changed: true };
  const i = colIndex(l, column);
  return { value: { kind: "orig", value: i < 0 ? null : (l.rows[row]?.[i] ?? null) }, changed: false };
}

/** `DataKey.columns` sırasıyla özgün anahtar değerleri; anahtar yoksa ya da bir değer null ise `[]`. */
export function keyOf(l: Loaded, row: number): string[] {
  const r = l.rows[row];
  const key = l.source.key;
  if (!r || !key) return [];
  const out: string[] = [];
  for (const name of key.columns) {
    const i = colIndex(l, name);
    const v = i < 0 ? null : r[i];
    if (typeof v !== "string") return [];
    out.push(v);
  }
  return out;
}

// --- Düzenleme işlemleri (işlem yoksa aynı `e` döner) -------------------------------------------

/** Özgün değere eşitse girdi kaldırılır; silinmiş/uygunsuz hücrede durum değişmez. */
export function setCell(l: Loaded, e: EditState, row: number, column: string, value: CellValue): EditState {
  if (!cellState(l, e, row, column).editable) return e;
  const orig = l.rows[row][colIndex(l, column)];
  return withCell(e, row, column, sameAsOriginal(orig, value) ? null : value);
}

export function revertCell(e: EditState, row: number, column: string): EditState {
  return withCell(e, row, column, null);
}

/** Yeni satır hücresi: dokunulan her değer (Default dahil) Insert'e girer. */
export function newRowSetCell(l: Loaded, e: EditState, id: number, column: string, value: CellValue): EditState {
  const st = cellState(l, e, { newId: id }, column);
  if (!st.editable && !(st.reason === "identityAlways" && value.kind === "default")) return e;
  const inserted = e.inserted.map((r) => {
    if (r.id !== id) return r;
    const values = new Map(r.values);
    values.set(column, value);
    return { id, values };
  });
  return { ...e, inserted };
}

/** Yeni satırın kolon girdisini kaldırır; tümü kalkan satır listede kalır ama yeniden atlanır. */
export function newRowRevertCell(e: EditState, id: number, column: string): EditState {
  const target = e.inserted.find((r) => r.id === id);
  if (!target?.values.has(column)) return e;
  const inserted = e.inserted.map((r) => {
    if (r.id !== id) return r;
    const values = new Map(r.values);
    values.delete(column);
    return { id, values };
  });
  return { ...e, inserted };
}

/** Silinecek ↔ geri al; silinen satırın düzenlemeleri tutulur ama gönderilmez. */
export function toggleDelete(l: Loaded, e: EditState, row: number): EditState {
  if (!writable(l) || !l.source.canDelete || !identity(l, row)) return e;
  const deleted = new Set(e.deleted);
  if (deleted.has(row)) deleted.delete(row);
  else deleted.add(row);
  return { ...e, deleted, order: syncRowSlot(e.order, row, e.cells, deleted) };
}

/** Boş yeni satır (kimlik artan); yazılamaz kaynakta ya da `canInsert=false` iken no-op. Dokunulmamış satır gönderilmez. */
export function addRow(l: Loaded, e: EditState): EditState {
  if (!writable(l) || !l.source.canInsert) return e;
  const id = e.nextId;
  return {
    ...e,
    inserted: [...e.inserted, { id, values: new Map() }],
    order: [...e.order, { kind: "new", newId: id }],
    nextId: id + 1,
  };
}

/** Yeni satırı listeden çıkarır (Delete üretilmez). */
export function removeNewRow(e: EditState, id: number): EditState {
  if (!e.inserted.some((r) => r.id === id)) return e;
  return {
    ...e,
    inserted: e.inserted.filter((r) => r.id !== id),
    order: e.order.filter((s) => !(s.kind === "new" && s.newId === id)),
  };
}

// --- Değişiklik üretimi --------------------------------------------------------------------------

/** Hücreleri kaynak kolon sırasıyla dizer. */
function ordered(l: Loaded, m: ReadonlyMap<string, CellValue>): CellEdit[] {
  const out: CellEdit[] = [];
  for (const c of l.source.columns) {
    const v = m.get(c.name);
    if (v) out.push({ column: c.name, value: v });
  }
  return out;
}

/**
 * Değişiklikler ilk dokunuş sırasıyla. Update/Delete `key`/`version` özgün satırdan; silinecek satır yalnız Delete;
 * dokunulmamış yeni satır atlanır. Yazılamayan kaynakta ve kimliksiz (anahtar/sürüm null) satırda değişiklik yok.
 */
export function toChangeSet(l: Loaded, e: EditState): { changes: RowChange[]; origin: ChangeOrigin[] } {
  const changes: RowChange[] = [];
  const origin: ChangeOrigin[] = [];
  if (!writable(l)) return { changes, origin };
  for (const s of e.order) {
    if (s.kind === "new") {
      const r = e.inserted.find((n) => n.id === s.newId);
      if (!r || r.values.size === 0) continue;
      changes.push({ op: "insert", values: ordered(l, r.values) });
      origin.push({ kind: "insert", newId: s.newId });
      continue;
    }
    const id = identity(l, s.rowIndex);
    if (!id) continue;
    if (e.deleted.has(s.rowIndex)) {
      changes.push({ op: "delete", key: id.key, version: id.version });
      origin.push({ kind: "delete", rowIndex: s.rowIndex });
      continue;
    }
    const m = e.cells.get(s.rowIndex);
    const set = m ? ordered(l, m) : [];
    if (set.length === 0) continue;
    changes.push({ op: "update", key: id.key, version: id.version, set });
    origin.push({ kind: "update", rowIndex: s.rowIndex });
  }
  return { changes, origin };
}

// --- Uygulama sonuçları --------------------------------------------------------------------------

/** `returned` = kaynak kolonları + sürüm (son); `versionIndex` kolon sayısına eşit değilse hepsi biçimsiz sayılır. */
function returnedRow(l: Loaded, results: readonly RowResult[], index: number): Row | null {
  const width = l.source.columns.length;
  if (l.versionIndex !== width) return null;
  const r = results.find((x) => x.index === index)?.returned;
  return r && r.length === width + 1 ? [...r] : null;
}

/**
 * Kullanılamayan sonuçların değişiklik indeksleri (INSERT/UPDATE `returned` yok ya da kolon sayısı tutmuyor;
 * sonuç hiç yok). Boş değilse denetleyici "yeniden yükle" önerir; `applyResults` bu satırları değiştirmez/eklemez.
 */
export function resultIssues(l: Loaded, origin: readonly ChangeOrigin[], results: readonly RowResult[]): number[] {
  const out: number[] = [];
  origin.forEach((o, i) => {
    if (o.kind === "delete") {
      if (!results.some((x) => x.index === i)) out.push(i);
    } else if (!returnedRow(l, results, i)) out.push(i);
  });
  return out;
}

/**
 * Başarılı uygulama sonrası yükleneni tazeler: UPDATE → satır yerinde (`returned`: kolonlar + yeni sürüm),
 * DELETE → satır çıkarılır (büyükten küçüğe; indeks kayması yanlış satırı silmez), INSERT → değişiklik sırasıyla
 * sona eklenir. `returned` kullanılamayan INSERT/UPDATE atlanır (bkz. `resultIssues`). DELETE'ler her zaman uygulanır
 * (uygulama ya hep ya hiç: `Applied` geldiyse silindi) — `resultIssues` sonucu eksik DELETE'i yine bildirebilir.
 * Düzenleme durumu çağıranca `EMPTY_EDIT`'e sıfırlanır.
 */
export function applyResults(l: Loaded, origin: readonly ChangeOrigin[], results: readonly RowResult[]): Loaded {
  const rows = [...l.rows];
  const deletes: number[] = [];
  const appended: Row[] = [];
  origin.forEach((o, i) => {
    if (o.kind === "delete") {
      deletes.push(o.rowIndex);
      return;
    }
    const r = returnedRow(l, results, i);
    if (!r) return;
    if (o.kind === "update") {
      if (o.rowIndex >= 0 && o.rowIndex < rows.length) rows[o.rowIndex] = r;
    } else appended.push(r);
  });
  for (const idx of [...new Set(deletes)].sort((a, b) => b - a)) {
    if (idx >= 0 && idx < rows.length) rows.splice(idx, 1);
  }
  return { ...l, rows: [...rows, ...appended] };
}
