// Veri ızgarası modeli (saf; Solid/DOM yok): genişlik, hücre gösterimi, odak hareketi, satır yuvaları, hücre türü ve
// komut sarmalayıcıları. Küçük yardımcılar `query/grid-model.ts`'ten KOPYADIR (Plan 17 Karar 2: o modül QueryTab
// parçasındadır; içe aktarılırsa paylaşılan parça doğar). Değerler günlüğe yazılmaz.
import { tdata } from "../../app/strings-data";
import type { CellValue } from "../../ipc/bindings/CellValue";
import type { DataSource } from "../../ipc/bindings/DataSource";
import { visibleControls, type DataGate } from "./data-apply-model";
import {
  cellState, cellValueAt, newRowRevertCell, newRowSetCell, removeNewRow, revertCell, setCell, toggleDelete,
  type ChangeOrigin, type EditState, type Loaded,
} from "./data-edit-model";
import type { FieldIssues } from "./data-errors";

export const ROW_HEIGHT = 24;
export const MIN_COL_WIDTH = 60;
export const MAX_COL_WIDTH = 480;
export const MAX_CELL_CHARS = 1000;
export const SAMPLE_ROWS = 200;
/** Bu uzunluğun üstündeki (ya da satır sonu içeren) değer `<textarea>` ile düzenlenir. */
export const INLINE_MAX = 120;
const CELL_PADDING = 24;

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

export interface TextMeasure {
  header(s: string): number;
  type(s: string): number;
  cell(s: string): number;
}

/** Ölçücü yoksa (ör. test) karakter başına yaklaşık genişlik. */
const APPROX: TextMeasure = { header: (s) => s.length * 7.5, type: (s) => s.length * 6.5, cell: (s) => s.length * 7.2 };

/** Sütun genişliği: başlık adı, tür etiketi ve örnek satırların en geniş görüntüsü (+ dolgu), 60..480 px. */
export function initialWidths(
  columns: readonly { name: string; typeDisplay: string }[],
  sampleRows: readonly (readonly (string | null)[])[],
  measure: TextMeasure = APPROX,
): number[] {
  return columns.map((col, c) => {
    let w = Math.max(measure.header(col.name), measure.type(col.typeDisplay));
    const n = Math.min(sampleRows.length, SAMPLE_ROWS);
    for (let r = 0; r < n; r++) {
      const v = sampleRows[r][c] ?? null;
      w = Math.max(w, measure.cell(v === null ? "NULL" : displayCell(v).text));
    }
    return clamp(Math.ceil(w) + CELL_PADDING, MIN_COL_WIDTH, MAX_COL_WIDTH);
  });
}

const NUMERIC = new Set([
  "int2", "int4", "int8", "smallint", "integer", "bigint", "numeric", "decimal", "float4", "float8", "real",
  "double precision", "oid", "money",
]);
/** Kısa ad (`int4`) ya da görünen ad (`numeric(10,2)`); diziler sayısal sayılmaz. */
export function isNumericType(typeName: string): boolean {
  const t = typeName.toLowerCase().trim();
  if (t.endsWith("]")) return false;
  return NUMERIC.has(t.replace(/\(.*\)$/, "").trim());
}

/** Hücre metni: ilk 1.000 karakter (vekil çift bölünmez), satır sonları ⏎, bidi/denetim karakterleri görünür kaçışlı. */
export function displayCell(v: string | null): { text: string; null: boolean; truncated: boolean } {
  if (v === null) return { text: "", null: true, truncated: false };
  let truncated = false;
  let s = v;
  if (s.length > MAX_CELL_CHARS) {
    let end = MAX_CELL_CHARS;
    const code = s.charCodeAt(end - 1);
    if (code >= 0xd800 && code <= 0xdbff) end--;
    s = s.slice(0, end);
    truncated = true;
  }
  s = visibleControls(s.replace(/\r\n|\r|\n/g, "⏎"));
  return { text: truncated ? s + "…" : s, null: false, truncated };
}

// ---- Odak ----
export interface Pos {
  r: number;
  c: number;
}
export type NavKey = "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Home" | "End" | "PageUp" | "PageDown";
export const NAV_KEYS: ReadonlySet<string> = new Set<NavKey>([
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown",
]);
/** Odağın yeni konumu (sınırlara kıstırılır). Ctrl+Home/End: ilk/son hücre. */
export function moveFocus(p: Pos, key: NavKey, rowCount: number, colCount: number, pageSize: number, ctrl = false): Pos {
  const lastR = Math.max(0, rowCount - 1);
  const lastC = Math.max(0, colCount - 1);
  let { r, c } = p;
  switch (key) {
    case "ArrowUp": r--; break;
    case "ArrowDown": r++; break;
    case "ArrowLeft": c--; break;
    case "ArrowRight": c++; break;
    case "PageUp": r -= Math.max(1, pageSize); break;
    case "PageDown": r += Math.max(1, pageSize); break;
    case "Home": c = 0; if (ctrl) r = 0; break;
    case "End": c = lastC; if (ctrl) r = lastR; break;
  }
  return { r: clamp(r, 0, lastR), c: clamp(c, 0, lastC) };
}

// ---- Satır yuvaları: önce yüklü satırlar, sonra yeni satırlar (alta) ----
export type GridSlot = { kind: "row"; rowIndex: number } | { kind: "new"; newId: number };

export function rowCountOf(l: Loaded, e: EditState): number {
  return l.rows.length + e.inserted.length;
}

export function slotAt(l: Loaded, e: EditState, r: number): GridSlot | null {
  if (r < 0) return null;
  if (r < l.rows.length) return { kind: "row", rowIndex: r };
  const n = e.inserted[r - l.rows.length];
  return n ? { kind: "new", newId: n.id } : null;
}

export const slotKey = (s: GridSlot): string => (s.kind === "row" ? `r${s.rowIndex}` : `n${s.newId}`);

const cellTarget = (s: GridSlot) => (s.kind === "row" ? s.rowIndex : { newId: s.newId });

/** Gösterilen değer; yeni satırın dokunulmamış hücresi sunucu varsayılanıdır (`default`, değişmedi). */
export function valueAt(l: Loaded, e: EditState, s: GridSlot, column: string): { value: CellValue; changed: boolean } {
  if (s.kind === "new") {
    const v = e.inserted.find((n) => n.id === s.newId)?.values.get(column);
    return v ? { value: v, changed: true } : { value: { kind: "default" }, changed: false };
  }
  const r = cellValueAt(l, e, s.rowIndex, column);
  if (r.value.kind !== "orig") return { value: r.value, changed: r.changed };
  return { value: r.value.value === null ? { kind: "null" } : { kind: "text", value: r.value.value }, changed: false };
}

/** Hücre düzenlenebilir mi (yeni satırda ALWAYS kimlik yalnızca DEFAULT kabul ettiğinden `defaultOnly`). */
export function editableAt(l: Loaded, e: EditState, s: GridSlot, column: string): { editable: boolean; defaultOnly: boolean } {
  const st = cellState(l, e, cellTarget(s), column);
  return { editable: st.editable, defaultOnly: !st.editable && s.kind === "new" && st.reason === "identityAlways" };
}

export function setValue(l: Loaded, e: EditState, s: GridSlot, column: string, v: CellValue): EditState {
  return s.kind === "row" ? setCell(l, e, s.rowIndex, column, v) : newRowSetCell(l, e, s.newId, column, v);
}

export function revertValue(e: EditState, s: GridSlot, column: string): EditState {
  return s.kind === "row" ? revertCell(e, s.rowIndex, column) : newRowRevertCell(e, s.newId, column);
}

/** Var olan satır: silinecek ↔ geri al; yeni satır: listeden çıkar. */
export function toggleRowDelete(l: Loaded, e: EditState, s: GridSlot): EditState {
  return s.kind === "row" ? toggleDelete(l, e, s.rowIndex) : removeNewRow(e, s.newId);
}

// ---- Hücre türü ve hatalar ----
export type CellKind = "normal" | "changed" | "null" | "default" | "readonly" | "error";

/** Yuva anahtarlı hata haritaları: hücre `${slotKey}:${kolon}`, satır `${slotKey}`. */
export interface IssueMaps {
  cell: Map<string, string[]>;
  row: Map<string, string[]>;
}

/** `fieldIssues` (değişiklik indeksli) → yuva anahtarlı; `origin` hataları üreten önizlemenin değişiklik sırasıdır. */
export function issueMaps(issues: FieldIssues | null, origin: readonly ChangeOrigin[]): IssueMaps {
  const out: IssueMaps = { cell: new Map(), row: new Map() };
  if (!issues) return out;
  const key = (i: number): string | null => {
    const o = origin[i];
    if (!o) return null;
    return o.kind === "insert" ? `n${o.newId}` : `r${o.rowIndex}`;
  };
  for (const [k, msgs] of issues.cell) {
    const sep = k.indexOf(":");
    const s = key(Number(k.slice(0, sep)));
    if (s) out.cell.set(`${s}:${k.slice(sep + 1)}`, msgs);
  }
  for (const [i, msgs] of issues.row) {
    const s = key(i);
    if (s) out.row.set(s, msgs);
  }
  return out;
}

/**
 * Görsel/`sr-only` için hücrenin baskın türü. Öncelik: hata > yazılmış DEFAULT > NULL > değişti > salt okunur >
 * (yeni satırın dokunulmamış hücresi) DEFAULT > normal.
 * (Değişmiş NULL/DEFAULT hücrenin "değişti" zemini ayrıca `valueAt().changed` ile çizilir.)
 */
export function cellKind(l: Loaded, e: EditState, s: GridSlot, column: string, errors?: ReadonlyMap<string, string[]>): CellKind {
  if (errors?.has(`${slotKey(s)}:${column}`)) return "error";
  const v = valueAt(l, e, s, column);
  if (v.changed && v.value.kind === "default") return "default";
  if (v.value.kind === "null") return "null";
  if (v.changed) return "changed";
  const st = cellState(l, e, cellTarget(s), column);
  if (!st.editable && st.reason !== "deleted") return "readonly";
  // Yeni satırın dokunulmamış (yazılabilir) hücresi: sunucu varsayılanı.
  return v.value.kind === "default" ? "default" : "normal";
}

// ---- Düzenleyici ----
/**
 * Düzenleyici sonucu → yazılacak değer; `null` = işlem yok. NULL/DEFAULT hücre düzenleyicide yazı değişmeden
 * onaylanırsa boş metne dönüşmez (NULL ≠ '').
 */
export function commitText(current: CellValue, initial: string, text: string): CellValue | null {
  if (current.kind !== "text" && text === initial) return null;
  return { kind: "text", value: text };
}

export const useTextarea = (text: string): boolean => text.length > INLINE_MAX || /[\r\n]/.test(text);

/** Ctrl+C metni: tam değer; NULL/DEFAULT boş metin. */
export const copyValue = (v: CellValue): string => (v.kind === "text" ? v.value : "");

// ---- Salt okunur / kapı gerekçeleri ----
/** Kaynağın salt okunur gerekçesi (kilit > kaynak gerekçesi > sürüm yok); düzenlenebilirse `null`. */
export function readOnlyText(source: DataSource, hasVersion: boolean, locked: boolean): string | null {
  if (locked || source.locked) return tdata.ro.locked;
  if (!source.editable) return source.readOnlyReason ? tdata.ro.reason[source.readOnlyReason] : tdata.status.readOnly;
  if (!hasVersion) return tdata.ro.noVersion;
  return null;
}

export interface EditBlockInput {
  locked: boolean;
  status: string;
  applying: boolean;
  outcomeUnknown: boolean;
  needsReload: boolean;
  loaded: Loaded | null;
}

/** Düzenleme komutları neden kapalı (`title`); açıksa `null`. */
export function editBlock(m: EditBlockInput): string | null {
  if (m.status === "disconnected" || m.status === "gone") return tdata.blocked.disconnected;
  if (!m.loaded || m.status !== "ready") return tdata.status.loading;
  const ro = readOnlyText(m.loaded.source, m.loaded.hasVersion, m.locked);
  if (ro) return ro;
  if (m.applying) return tdata.blocked.busy;
  if (m.outcomeUnknown) return tdata.blocked.unknown;
  if (m.needsReload) return tdata.gate.planChanged;
  return null;
}

/** Kaydet düğmesinin kapalı gerekçesi (`title`); gönderilebilirse `null`. */
export function gateReason(g: DataGate, checked: boolean): string | null {
  if (g.kind === "ready") return null;
  if (g.kind === "needsConfirm") return checked ? null : tdata.confirm.needed;
  switch (g.reason) {
    case "locked": return tdata.blocked.locked;
    case "readOnly": return tdata.blocked.readOnly;
    case "empty": return tdata.blocked.empty;
    case "invalid": return tdata.blocked.invalid;
    case "pending": return tdata.blocked.pending;
    case "applying": return tdata.blocked.busy;
    case "disconnected": return tdata.blocked.disconnected;
    case "unknown": return tdata.blocked.unknown;
    case "stale": return tdata.gate.planChanged;
    case "loading": return tdata.status.loading;
  }
}
