import { describe, expect, it } from "vitest";
import { tdata } from "../../app/strings-data";
import type { CellValue } from "../../ipc/bindings/CellValue";
import type { DataColumn } from "../../ipc/bindings/DataColumn";
import type { DataSource } from "../../ipc/bindings/DataSource";
import { EMPTY_EDIT, addRow, newRowSetCell, setCell, toChangeSet, toggleDelete, type Loaded } from "./data-edit-model";
import {
  MAX_CELL_CHARS, MAX_COL_WIDTH, MIN_COL_WIDTH, ROW_HEIGHT, cellKind, commitText, copyValue, displayCell, editBlock,
  gateReason, initialWidths, isNumericType, issueMaps, moveFocus, readOnlyText, revertValue, rowCountOf, setValue, slotAt,
  slotKey, toggleRowDelete, useTextarea, valueAt,
} from "./data-grid-model";

// Fikstürler yapay veridir (gerçek veri yok).
const T = (value: string): CellValue => ({ kind: "text", value });
const NULL: CellValue = { kind: "null" };
const DEF: CellValue = { kind: "default" };

function col(attnum: number, name: string, over: Partial<DataColumn> = {}): DataColumn {
  return {
    attnum, name, typeOid: 25, typeDisplay: "text", notNull: false, hasDefault: false, identity: "none", generated: false,
    insertable: true, updatable: true, ...over,
  };
}

function source(over: Partial<DataSource> = {}): DataSource {
  return {
    object: { kind: "table", oid: 1 }, schema: "public", name: "t", relkind: "table",
    columns: [col(1, "id", { typeDisplay: "integer" }), col(2, "name"), col(3, "gen", { generated: true, insertable: false, updatable: false })],
    key: { source: "primary_key", name: "t_pkey", columns: ["id"] }, canSelect: true, canInsert: true, canUpdate: true,
    canDelete: true, locked: false, editable: true, readOnlyReason: null, fingerprint: "fp", ...over,
  };
}

const loaded = (over: Partial<DataSource> = {}): Loaded => ({
  source: source(over),
  rows: [
    ["1", "a", "g1", "v1"],
    ["2", null, "g2", "v2"],
  ],
  versionIndex: 3,
  hasVersion: true,
});

describe("data-grid-model", () => {
  it("sabitler", () => {
    expect(ROW_HEIGHT).toBe(24);
    expect(MAX_CELL_CHARS).toBe(1000);
  });

  it("displayCell: NULL, boş, uzun, çok satırlı, bidi/denetim kaçışı", () => {
    expect(displayCell(null)).toEqual({ text: "", null: true, truncated: false });
    expect(displayCell("")).toEqual({ text: "", null: false, truncated: false });
    const long = displayCell("x".repeat(1500));
    expect(long.truncated).toBe(true);
    expect(long.text).toBe(`${"x".repeat(1000)}…`);
    // Vekil çift ortadan kesilmez.
    const pair = displayCell(`${"x".repeat(999)}😀yyy`);
    expect(pair.text).toBe(`${"x".repeat(999)}…`);
    expect(displayCell("a\r\nb\nc\rd").text).toBe("a⏎b⏎c⏎d");
    expect(displayCell("ab‮cd⁦e\u0000").text).toBe("ab⟦U+202E⟧cd⟦U+2066⟧e⟦U+0000⟧");
    expect(displayCell("tab\tok").text).toBe("tab\tok");
  });

  it("isNumericType: kısa ad ve görünen ad; dizi sayısal değil", () => {
    for (const t of ["int4", "integer", "bigint", "smallint", "numeric(10,2)", "double precision", "real", "money", "oid", "NUMERIC"]) {
      expect(isNumericType(t)).toBe(true);
    }
    for (const t of ["text", "integer[]", "character varying(20)", "timestamp with time zone", "uuid"]) {
      expect(isNumericType(t)).toBe(false);
    }
  });

  it("moveFocus: sınırlar, sayfa, Home/End (+Ctrl)", () => {
    expect(moveFocus({ r: 0, c: 0 }, "ArrowUp", 10, 3, 5)).toEqual({ r: 0, c: 0 });
    expect(moveFocus({ r: 0, c: 0 }, "ArrowLeft", 10, 3, 5)).toEqual({ r: 0, c: 0 });
    expect(moveFocus({ r: 9, c: 2 }, "ArrowDown", 10, 3, 5)).toEqual({ r: 9, c: 2 });
    expect(moveFocus({ r: 9, c: 2 }, "ArrowRight", 10, 3, 5)).toEqual({ r: 9, c: 2 });
    expect(moveFocus({ r: 2, c: 1 }, "PageDown", 10, 3, 5)).toEqual({ r: 7, c: 1 });
    expect(moveFocus({ r: 7, c: 1 }, "PageDown", 10, 3, 5)).toEqual({ r: 9, c: 1 });
    expect(moveFocus({ r: 3, c: 1 }, "PageUp", 10, 3, 5)).toEqual({ r: 0, c: 1 });
    expect(moveFocus({ r: 3, c: 1 }, "PageDown", 10, 3, 0)).toEqual({ r: 4, c: 1 });
    expect(moveFocus({ r: 3, c: 1 }, "Home", 10, 3, 5)).toEqual({ r: 3, c: 0 });
    expect(moveFocus({ r: 3, c: 1 }, "End", 10, 3, 5)).toEqual({ r: 3, c: 2 });
    expect(moveFocus({ r: 3, c: 1 }, "Home", 10, 3, 5, true)).toEqual({ r: 0, c: 0 });
    expect(moveFocus({ r: 3, c: 1 }, "End", 10, 3, 5, true)).toEqual({ r: 9, c: 2 });
    expect(moveFocus({ r: 0, c: 0 }, "ArrowDown", 0, 0, 5)).toEqual({ r: 0, c: 0 });
  });

  it("initialWidths: alt/üst sınırlar, başlık ve tür kırpılmaz", () => {
    const m = { header: (s: string) => s.length * 8, type: (s: string) => s.length * 6, cell: (s: string) => s.length * 7 };
    const cols = [
      { name: "a", typeDisplay: "int" },
      { name: "uzun_bir_kolon_adi", typeDisplay: "text" },
      { name: "b", typeDisplay: "text" },
    ];
    const w = initialWidths(cols, [[null, "x", "y".repeat(500)]], m);
    expect(w[0]).toBe(MIN_COL_WIDTH);
    expect(w[1]).toBe(18 * 8 + 24);
    expect(w[2]).toBe(MAX_COL_WIDTH);
    // Ölçücüsüz yaklaşık ölçüm de sınırlar içinde.
    for (const x of initialWidths(cols, [])) {
      expect(x).toBeGreaterThanOrEqual(MIN_COL_WIDTH);
      expect(x).toBeLessThanOrEqual(MAX_COL_WIDTH);
    }
  });

  it("slotAt/rowCountOf: yüklü satırlar, sonra yeni satırlar", () => {
    const l = loaded();
    const e = addRow(l, addRow(l, EMPTY_EDIT));
    expect(rowCountOf(l, e)).toBe(4);
    expect(slotAt(l, e, 1)).toEqual({ kind: "row", rowIndex: 1 });
    expect(slotAt(l, e, 2)).toEqual({ kind: "new", newId: 1 });
    expect(slotAt(l, e, 3)).toEqual({ kind: "new", newId: 2 });
    expect(slotAt(l, e, 4)).toBeNull();
    expect(slotKey({ kind: "row", rowIndex: 3 })).toBe("r3");
    expect(slotKey({ kind: "new", newId: 2 })).toBe("n2");
  });

  it("valueAt/setValue/revertValue/toggleRowDelete: var olan ve yeni satır", () => {
    const l = loaded();
    const r0 = { kind: "row" as const, rowIndex: 0 };
    expect(valueAt(l, EMPTY_EDIT, r0, "name")).toEqual({ value: T("a"), changed: false });
    expect(valueAt(l, EMPTY_EDIT, { kind: "row", rowIndex: 1 }, "name")).toEqual({ value: NULL, changed: false });
    let e = setValue(l, EMPTY_EDIT, r0, "name", NULL);
    expect(valueAt(l, e, r0, "name")).toEqual({ value: NULL, changed: true });
    e = revertValue(e, r0, "name");
    expect(e.cells.size).toBe(0);
    e = toggleRowDelete(l, e, r0);
    expect(e.deleted.has(0)).toBe(true);

    let n = addRow(l, EMPTY_EDIT);
    const s = { kind: "new" as const, newId: 1 };
    // Dokunulmamış yeni satır hücresi sunucu varsayılanıdır (değişmedi).
    expect(valueAt(l, n, s, "name")).toEqual({ value: DEF, changed: false });
    n = setValue(l, n, s, "name", T("z"));
    expect(valueAt(l, n, s, "name")).toEqual({ value: T("z"), changed: true });
    n = revertValue(n, s, "name");
    expect(valueAt(l, n, s, "name").changed).toBe(false);
    n = toggleRowDelete(l, n, s);
    expect(n.inserted).toHaveLength(0);
  });

  it("cellKind tablosu: değişti / NULL / DEFAULT / salt okunur / hata / normal", () => {
    const l = loaded();
    const r0 = { kind: "row" as const, rowIndex: 0 };
    const r1 = { kind: "row" as const, rowIndex: 1 };
    expect(cellKind(l, EMPTY_EDIT, r0, "name")).toBe("normal");
    expect(cellKind(l, EMPTY_EDIT, r1, "name")).toBe("null");
    expect(cellKind(l, EMPTY_EDIT, r0, "gen")).toBe("readonly");
    let e = setCell(l, EMPTY_EDIT, 0, "name", T("b"));
    expect(cellKind(l, e, r0, "name")).toBe("changed");
    e = setCell(l, e, 1, "name", DEF);
    expect(cellKind(l, e, r1, "name")).toBe("default");
    e = setCell(l, e, 0, "id", NULL);
    expect(cellKind(l, e, r0, "id")).toBe("null");
    const errs = new Map([["r0:name", ["hatalı"]]]);
    expect(cellKind(l, e, r0, "name", errs)).toBe("error");
    // Salt okunur kaynakta her hücre salt okunur (NULL görünümü önce gelir).
    const ro = loaded({ editable: false, readOnlyReason: "no_key" });
    expect(cellKind(ro, EMPTY_EDIT, r0, "name")).toBe("readonly");
    expect(cellKind(ro, EMPTY_EDIT, r1, "name")).toBe("null");
    // Yeni satır: dokunulmamış hücre DEFAULT, üretilmiş kolon salt okunur.
    const n = addRow(l, EMPTY_EDIT);
    expect(cellKind(l, n, { kind: "new", newId: 1 }, "name")).toBe("default");
    expect(cellKind(l, n, { kind: "new", newId: 1 }, "gen")).toBe("readonly");
  });

  it("issueMaps: değişiklik indeksi → satır/yeni satır anahtarı", () => {
    const l = loaded();
    let e = setCell(l, EMPTY_EDIT, 1, "name", T("q"));
    e = addRow(l, e);
    e = newRowSetCell(l, e, 1, "name", T("w"));
    e = toggleDelete(l, e, 0);
    const { origin } = toChangeSet(l, e);
    const issues = {
      cell: new Map([["0:name", ["c0"]], ["1:name", ["c1"]]]),
      row: new Map([[2, ["r2"]]]),
      general: ["g"],
    };
    const m = issueMaps(issues, origin);
    expect(m.cell.get("r1:name")).toEqual(["c0"]);
    expect(m.cell.get("n1:name")).toEqual(["c1"]);
    expect(m.row.get("r0")).toEqual(["r2"]);
    expect(issueMaps(null, origin).cell.size).toBe(0);
  });

  it("commitText: değişmeyen NULL/DEFAULT boş metne dönmez; metin seçimi", () => {
    expect(commitText(NULL, "", "")).toBeNull();
    expect(commitText(DEF, "", "")).toBeNull();
    expect(commitText(NULL, "", "x")).toEqual(T("x"));
    expect(commitText(T("a"), "a", "a")).toEqual(T("a"));
    expect(commitText(T("a"), "a", "")).toEqual(T(""));
    expect(useTextarea("a\nb")).toBe(true);
    expect(useTextarea("x".repeat(121))).toBe(true);
    expect(useTextarea("x".repeat(120))).toBe(false);
    expect(copyValue(NULL)).toBe("");
    expect(copyValue(DEF)).toBe("");
    expect(copyValue(T("a\nb"))).toBe("a\nb");
  });

  it("readOnlyText ve editBlock: kilit, gerekçe, sürüm yok, durum", () => {
    expect(readOnlyText(source(), true, false)).toBeNull();
    expect(readOnlyText(source(), true, true)).toBe(tdata.ro.locked);
    expect(readOnlyText(source({ editable: false, readOnlyReason: "no_privilege" }), true, false)).toBe(tdata.ro.reason.no_privilege);
    expect(readOnlyText(source(), false, false)).toBe(tdata.ro.noVersion);
    const l = loaded();
    const base = { locked: false, status: "ready" as const, applying: false, outcomeUnknown: false, needsReload: false, loaded: l };
    expect(editBlock(base)).toBeNull();
    expect(editBlock({ ...base, locked: true })).toBe(tdata.ro.locked);
    expect(editBlock({ ...base, applying: true })).toBe(tdata.blocked.busy);
    expect(editBlock({ ...base, outcomeUnknown: true })).toBe(tdata.blocked.unknown);
    expect(editBlock({ ...base, needsReload: true })).toBe(tdata.gate.planChanged);
    expect(editBlock({ ...base, status: "disconnected" })).toBe(tdata.blocked.disconnected);
    expect(editBlock({ ...base, loaded: null })).toBe(tdata.status.loading);
  });

  it("gateReason: kapı gerekçesi; onay bekleyen", () => {
    expect(gateReason({ kind: "ready" }, false)).toBeNull();
    expect(gateReason({ kind: "needsConfirm", production: false, destructive: true }, false)).toBe(tdata.confirm.needed);
    expect(gateReason({ kind: "needsConfirm", production: true, destructive: false }, true)).toBeNull();
    expect(gateReason({ kind: "blocked", reason: "empty" }, false)).toBe(tdata.blocked.empty);
    expect(gateReason({ kind: "blocked", reason: "stale" }, false)).toBe(tdata.gate.planChanged);
    expect(gateReason({ kind: "blocked", reason: "applying" }, false)).toBe(tdata.blocked.busy);
    expect(gateReason({ kind: "blocked", reason: "loading" }, false)).toBe(tdata.status.loading);
  });
});
