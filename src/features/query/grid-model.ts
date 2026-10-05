// Sonuç ızgarası modeli (saf): genişlik, hücre gösterimi, seçim ve TSV kopyalama.
import type { Column } from "../../ipc/bindings/Column";

export const ROW_HEIGHT = 24;
export const MIN_COL_WIDTH = 60;
export const MAX_COL_WIDTH = 480;
export const MAX_CELL_CHARS = 1000;
export const SAMPLE_ROWS = 200;
const CELL_PADDING = 24;

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

/** Sütun genişliği: başlık ve örnek satırların en geniş görüntüsü (+ dolgu), 60..480 px. */
export function initialWidths(columns: Column[], sampleRows: (string | null)[][], measure: (s: string) => number): number[] {
  return columns.map((col, c) => {
    let w = measure(col.name);
    const n = Math.min(sampleRows.length, SAMPLE_ROWS);
    for (let r = 0; r < n; r++) {
      const v = sampleRows[r][c];
      w = Math.max(w, measure(v === null ? "[NULL]" : displayCell(v).text));
    }
    return clamp(Math.ceil(w) + CELL_PADDING, MIN_COL_WIDTH, MAX_COL_WIDTH);
  });
}

const NUMERIC = new Set(["int2", "int4", "int8", "numeric", "float4", "float8", "oid", "money"]);
export function isNumericType(typeName: string): boolean {
  return NUMERIC.has(typeName.toLowerCase());
}

/** Hücre metni: ilk 1.000 karakter, satır sonları ⏎. NULL ile boş metin ayrıdır. */
export function displayCell(v: string | null): { text: string; null: boolean; truncated: boolean } {
  if (v === null) return { text: "", null: true, truncated: false };
  let truncated = false;
  let s = v;
  if (s.length > MAX_CELL_CHARS) {
    // Vekil çiftini ortadan kesmemek için.
    let end = MAX_CELL_CHARS;
    const code = s.charCodeAt(end - 1);
    if (code >= 0xd800 && code <= 0xdbff) end--;
    s = s.slice(0, end);
    truncated = true;
  }
  s = s.replace(/\r\n|\r|\n/g, "⏎");
  return { text: truncated ? s + "…" : s, null: false, truncated };
}

// ---- Seçim ----
export interface Pos {
  r: number;
  c: number;
}
/** `anchor`: seçimin sabit ucu; `focus`: klavye odağı / genişleyen uç. */
export interface Selection {
  anchor: Pos;
  focus: Pos;
}
export interface Rect {
  r0: number;
  r1: number;
  c0: number;
  c1: number;
}

export function selRect(s: Selection): Rect {
  return {
    r0: Math.min(s.anchor.r, s.focus.r),
    r1: Math.max(s.anchor.r, s.focus.r),
    c0: Math.min(s.anchor.c, s.focus.c),
    c1: Math.max(s.anchor.c, s.focus.c),
  };
}
export function inRect(rect: Rect, r: number, c: number): boolean {
  return r >= rect.r0 && r <= rect.r1 && c >= rect.c0 && c <= rect.c1;
}
export function selectCell(r: number, c: number): Selection {
  return { anchor: { r, c }, focus: { r, c } };
}
export function selectRow(r: number, colCount: number): Selection {
  return { anchor: { r, c: 0 }, focus: { r, c: Math.max(0, colCount - 1) } };
}
export function selectColumn(c: number, rowCount: number): Selection {
  return { anchor: { r: 0, c }, focus: { r: Math.max(0, rowCount - 1), c } };
}
export function selectAll(rowCount: number, colCount: number): Selection {
  return { anchor: { r: 0, c: 0 }, focus: { r: Math.max(0, rowCount - 1), c: Math.max(0, colCount - 1) } };
}
/** Shift ile genişletme: sabit uç korunur, odak `to`ya gider. */
export function extendTo(s: Selection, to: Pos): Selection {
  return { anchor: s.anchor, focus: to };
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

// ---- TSV ----
function tsvField(v: string | null): string {
  if (v === null) return "";
  return /[\t\r\n"]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

/** Seçili dikdörtgeni TSV yapar; sekme/satır sonu/tırnak içeren değerler tırnaklanır, NULL boş olur. */
export function selectionToTsv(
  rows: (string | null)[][],
  columns: Column[],
  sel: Rect,
  opts: { headers: boolean },
): string {
  const lines: string[] = [];
  const c1 = Math.min(sel.c1, columns.length - 1);
  if (opts.headers) {
    const h: string[] = [];
    for (let c = sel.c0; c <= c1; c++) h.push(tsvField(columns[c].name));
    lines.push(h.join("\t"));
  }
  const r1 = Math.min(sel.r1, rows.length - 1);
  for (let r = sel.r0; r <= r1; r++) {
    const line: string[] = [];
    for (let c = sel.c0; c <= c1; c++) line.push(tsvField(rows[r][c] ?? null));
    lines.push(line.join("\t"));
  }
  return lines.join("\n");
}

// ---- Hücre paneli ----
export const CELL_PANEL_MAX = 1024 * 1024;
export function isJsonType(typeName: string): boolean {
  const t = typeName.toLowerCase();
  return t === "json" || t === "jsonb";
}
/** Panel metni: JSON ise güzel yazdırılır; 1 MB üstü kırpılır (`clipped`). Kırpılmış değer ayrıştırılmaz. */
export function cellPanelText(v: string, typeName: string): { text: string; clipped: boolean } {
  if (v.length > CELL_PANEL_MAX) return { text: v.slice(0, CELL_PANEL_MAX), clipped: true };
  const trimmed = v.trimStart();
  if (isJsonType(typeName) || trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const parsed: unknown = JSON.parse(v);
      if (typeof parsed === "object" && parsed !== null) {
        const pretty = JSON.stringify(parsed, null, 2);
        return pretty.length > CELL_PANEL_MAX
          ? { text: pretty.slice(0, CELL_PANEL_MAX), clipped: true }
          : { text: pretty, clipped: false };
      }
    } catch {
      /* JSON değil: ham metin */
    }
  }
  return { text: v, clipped: false };
}
