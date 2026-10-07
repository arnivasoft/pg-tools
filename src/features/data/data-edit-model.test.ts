import { describe, expect, it } from "vitest";
import type { CellValue } from "../../ipc/bindings/CellValue";
import type { DataColumn } from "../../ipc/bindings/DataColumn";
import type { DataSource } from "../../ipc/bindings/DataSource";
import type { RowResult } from "../../ipc/bindings/RowResult";
import {
  EMPTY_EDIT,
  addRow,
  applyResults,
  cellState,
  cellValueAt,
  changeCount,
  isDirty,
  keyOf,
  newRowSetCell,
  removeNewRow,
  resultIssues,
  revertCell,
  setCell,
  toChangeSet,
  toggleDelete,
  type EditState,
  type Loaded,
} from "./data-edit-model";

// Fikstürler yapay veridir (gerçek veri yok).
const T = (value: string): CellValue => ({ kind: "text", value });
const NULL: CellValue = { kind: "null" };
const DEF: CellValue = { kind: "default" };

function col(attnum: number, name: string, over: Partial<DataColumn> = {}): DataColumn {
  return {
    attnum,
    name,
    typeOid: 25,
    typeDisplay: "text",
    notNull: false,
    hasDefault: false,
    identity: "none",
    generated: false,
    insertable: true,
    updatable: true,
    ...over,
  };
}

function deepFreeze<T>(o: T): T {
  if (o && typeof o === "object" && !Object.isFrozen(o)) {
    Object.freeze(o);
    for (const v of Object.values(o as object)) deepFreeze(v);
  }
  return o;
}

const COLUMNS: DataColumn[] = [
  col(1, "id", { typeOid: 23, typeDisplay: "integer", notNull: true }),
  col(2, "name"),
  col(3, "note", { hasDefault: true }),
  col(4, "gen", { generated: true, insertable: false, updatable: false }),
  col(5, "ident", { identity: "always", notNull: true, hasDefault: true, updatable: false }),
  col(6, "bd", { identity: "by_default", notNull: true, hasDefault: true }),
];

function source(over: Partial<DataSource> = {}): DataSource {
  return {
    object: { kind: "table", oid: 16384 },
    schema: "public",
    name: "t",
    relkind: "table",
    columns: COLUMNS,
    key: { source: "primary_key", name: "t_pkey", columns: ["id"] },
    canSelect: true,
    canInsert: true,
    canUpdate: true,
    canDelete: true,
    locked: false,
    editable: true,
    readOnlyReason: null,
    fingerprint: "0123456789abcdef",
    ...over,
  };
}

// id, name, note, gen, ident, bd, sürüm
const ROWS: (string | null)[][] = [
  ["1", "a", null, "g1", "11", "21", "100"],
  ["2", "b", "x", "g2", "12", "22", "101"],
  ["3", "c", "NULL", "g3", "13", "23", "102"],
  ["4", "d", "", "g4", "14", "24", "103"],
];

function loaded(over: Partial<DataSource> = {}, rows = ROWS, hasVersion = true): Loaded {
  return deepFreeze({ source: source(over), rows: rows.map((r) => [...r]), versionIndex: 6, hasVersion });
}

const L = loaded();

/** Durumu düz nesneye çevirir (Map/Set karşılaştırması için). */
function snap(e: EditState): unknown {
  return {
    cells: [...e.cells].map(([r, m]) => [r, [...m]]),
    inserted: e.inserted.map((n) => ({ id: n.id, values: [...n.values] })),
    deleted: [...e.deleted],
    order: e.order,
    nextId: e.nextId,
  };
}

describe("data-edit-model", () => {
  it("setCell özgün değere dönünce girdi kalkar; changeCount no-op saymaz", () => {
    expect(isDirty(EMPTY_EDIT)).toBe(false);
    const e1 = setCell(L, EMPTY_EDIT, 0, "name", T("z"));
    expect(changeCount(e1)).toBe(1);
    expect(isDirty(e1)).toBe(true);
    expect(cellValueAt(L, e1, 0, "name")).toEqual({ value: T("z"), changed: true });
    const e2 = setCell(L, e1, 0, "name", T("a"));
    expect(changeCount(e2)).toBe(0);
    expect(isDirty(e2)).toBe(false);
    expect(e2.cells.size).toBe(0);
    expect(e2.order).toEqual([]);
    expect(cellValueAt(L, e2, 0, "name")).toEqual({ value: { kind: "orig", value: "a" }, changed: false });
    // Aynı satırda iki hücre tek değişiklik
    const e3 = setCell(L, setCell(L, EMPTY_EDIT, 1, "name", T("q")), 1, "note", NULL);
    expect(changeCount(e3)).toBe(1);
    // revertCell
    const e4 = revertCell(e3, 1, "name");
    expect(cellValueAt(L, e4, 1, "name").changed).toBe(false);
    expect(changeCount(e4)).toBe(1);
    expect(changeCount(revertCell(e4, 1, "note"))).toBe(0);
    expect(revertCell(EMPTY_EDIT, 0, "name")).toBe(EMPTY_EDIT);
    // Özgün değere eşit ilk yazım durum değiştirmez
    expect(setCell(L, EMPTY_EDIT, 0, "name", T("a"))).toBe(EMPTY_EDIT);
  });

  it("Null/Text/Default ayrımı: '' ≠ null ≠ 'NULL' ≠ 'DEFAULT'; özgün null'a Null yazmak no-op", () => {
    // satır 0: note = null
    expect(setCell(L, EMPTY_EDIT, 0, "note", NULL)).toBe(EMPTY_EDIT);
    expect(changeCount(setCell(L, EMPTY_EDIT, 0, "note", T("")))).toBe(1);
    expect(changeCount(setCell(L, EMPTY_EDIT, 0, "note", T("NULL")))).toBe(1);
    expect(changeCount(setCell(L, EMPTY_EDIT, 0, "note", T("DEFAULT")))).toBe(1);
    // satır 2: note = 'NULL' metni
    expect(changeCount(setCell(L, EMPTY_EDIT, 2, "note", NULL))).toBe(1);
    expect(setCell(L, EMPTY_EDIT, 2, "note", T("NULL"))).toBe(EMPTY_EDIT);
    // satır 3: note = ''
    expect(changeCount(setCell(L, EMPTY_EDIT, 3, "note", NULL))).toBe(1);
    expect(setCell(L, EMPTY_EDIT, 3, "note", T(""))).toBe(EMPTY_EDIT);
    // Default her zaman değişiklik; UPDATE'te sıradan kolonda serbest
    const d = setCell(L, EMPTY_EDIT, 1, "note", DEF);
    expect(changeCount(d)).toBe(1);
    expect(toChangeSet(L, d).changes).toEqual([
      { op: "update", key: ["2"], version: "101", set: [{ column: "note", value: DEF }] },
    ]);
    expect(changeCount(setCell(L, EMPTY_EDIT, 1, "bd", DEF))).toBe(1);
    // Default üretilmiş / ALWAYS kimlik / güncellenemez kolonda reddedilir
    expect(setCell(L, EMPTY_EDIT, 1, "gen", DEF)).toBe(EMPTY_EDIT);
    expect(setCell(L, EMPTY_EDIT, 1, "ident", DEF)).toBe(EMPTY_EDIT);
    const nu = loaded({ columns: COLUMNS.map((c) => (c.name === "name" ? { ...c, updatable: false } : c)) });
    expect(setCell(nu, EMPTY_EDIT, 1, "name", DEF)).toBe(EMPTY_EDIT);
  });

  it("toChangeSet Update: set yalnız değişen kolonlar; key ve version özgün satırdan; anahtar kolonu düzenlense bile key eski değer", () => {
    let e = setCell(L, EMPTY_EDIT, 1, "note", T("y"));
    e = setCell(L, e, 1, "id", T("99"));
    e = setCell(L, e, 1, "name", T("tmp"));
    e = setCell(L, e, 1, "name", T("b")); // özgüne döndü → set'te yok
    const { changes, origin } = toChangeSet(L, e);
    expect(changes).toEqual([
      {
        op: "update",
        key: ["2"],
        version: "101",
        // kaynak kolon sırasıyla
        set: [
          { column: "id", value: T("99") },
          { column: "note", value: T("y") },
        ],
      },
    ]);
    expect(origin).toEqual([{ kind: "update", rowIndex: 1 }]);
    expect(keyOf(L, 1)).toEqual(["2"]);
  });

  it("toChangeSet Delete: key+version; silinmiş satırın düzenlemeleri gönderilmez; geri alınca Update'e döner", () => {
    let e = setCell(L, EMPTY_EDIT, 2, "name", T("w"));
    e = toggleDelete(L, e, 2);
    expect(changeCount(e)).toBe(1);
    expect(toChangeSet(L, e)).toEqual({
      changes: [{ op: "delete", key: ["3"], version: "102" }],
      origin: [{ kind: "delete", rowIndex: 2 }],
    });
    // Silinmiş satır düzenlenemez
    expect(setCell(L, e, 2, "note", T("k"))).toBe(e);
    expect(cellState(L, e, 2, "name")).toEqual({ editable: false, reason: "deleted" });
    // Düzenleme tutulur
    expect(cellValueAt(L, e, 2, "name")).toEqual({ value: T("w"), changed: true });
    const back = toggleDelete(L, e, 2);
    expect(toChangeSet(L, back).changes).toEqual([
      { op: "update", key: ["3"], version: "102", set: [{ column: "name", value: T("w") }] },
    ]);
    // Düzenlemesiz satırı sil + geri al → temiz
    const clean = toggleDelete(L, toggleDelete(L, EMPTY_EDIT, 0), 0);
    expect(changeCount(clean)).toBe(0);
    expect(clean.order).toEqual([]);
    // Silme yetkisi yoksa no-op
    const nd = loaded({ canDelete: false });
    expect(toggleDelete(nd, EMPTY_EDIT, 0)).toBe(EMPTY_EDIT);
  });

  it("toChangeSet Insert: yalnız dokunulan kolonlar; hiç dokunulmamış yeni satır atlanır; sırası değişiklik sırasıyla (insert/update/delete karışık) korunur ve origin eşler", () => {
    let e = toggleDelete(L, EMPTY_EDIT, 3); // 1. delete
    e = addRow(e); // id 1 — dokunulmamış, atlanır
    e = addRow(e); // id 2
    e = newRowSetCell(L, e, 2, "name", T("n"));
    e = newRowSetCell(L, e, 2, "id", T("5"));
    e = setCell(L, e, 0, "name", T("u")); // 3. update
    e = addRow(e); // id 3 — yalnız Default ile dokunulmuş
    e = newRowSetCell(L, e, 3, "note", DEF);
    e = setCell(L, e, 0, "note", T("v")); // aynı satır: sıra ilk dokunuşta kalır
    expect(e.inserted.map((n) => n.id)).toEqual([1, 2, 3]);
    expect(changeCount(e)).toBe(4);
    const { changes, origin } = toChangeSet(L, e);
    expect(changes).toEqual([
      { op: "delete", key: ["4"], version: "103" },
      {
        op: "insert",
        values: [
          { column: "id", value: T("5") },
          { column: "name", value: T("n") },
        ],
      },
      {
        op: "update",
        key: ["1"],
        version: "100",
        set: [
          { column: "name", value: T("u") },
          { column: "note", value: T("v") },
        ],
      },
      { op: "insert", values: [{ column: "note", value: DEF }] },
    ]);
    expect(origin).toEqual([
      { kind: "delete", rowIndex: 3 },
      { kind: "insert", newId: 2 },
      { kind: "update", rowIndex: 0 },
      { kind: "insert", newId: 3 },
    ]);
    // Yeni satır silinince listeden çıkar; Delete üretilmez
    const r = removeNewRow(e, 2);
    expect(r.inserted.map((n) => n.id)).toEqual([1, 3]);
    expect(toChangeSet(L, r).changes.map((c) => c.op)).toEqual(["delete", "update", "insert"]);
    expect(removeNewRow(e, 42)).toBe(e);
    // Kaldırılan id yeniden kullanılmaz
    expect(addRow(r).inserted.map((n) => n.id)).toEqual([1, 3, 4]);
    // Hiç dokunulmamış tek yeni satır: değişiklik yok, kirli değil
    const blank = addRow(EMPTY_EDIT);
    expect(changeCount(blank)).toBe(0);
    expect(isDirty(blank)).toBe(false);
    expect(toChangeSet(L, blank)).toEqual({ changes: [], origin: [] });
  });

  it("cellState: generated, identity always (Insert'te yalnız Default), updatable=false, silinmiş satır, salt okunur kaynak, view/MV (editable=false)", () => {
    expect(cellState(L, EMPTY_EDIT, 0, "name")).toEqual({ editable: true });
    expect(cellState(L, EMPTY_EDIT, 0, "bd")).toEqual({ editable: true });
    expect(cellState(L, EMPTY_EDIT, 0, "gen")).toEqual({ editable: false, reason: "generated" });
    expect(cellState(L, EMPTY_EDIT, 0, "ident")).toEqual({ editable: false, reason: "identityAlways" });
    const nu = loaded({ columns: COLUMNS.map((c) => (c.name === "name" ? { ...c, updatable: false } : c)) });
    expect(cellState(nu, EMPTY_EDIT, 0, "name")).toEqual({ editable: false, reason: "notUpdatable" });
    expect(cellState(loaded({ canUpdate: false }), EMPTY_EDIT, 0, "name")).toEqual({
      editable: false,
      reason: "notUpdatable",
    });
    expect(cellState(L, toggleDelete(L, EMPTY_EDIT, 0), 0, "name")).toEqual({ editable: false, reason: "deleted" });

    // Yeni satır
    const e = addRow(EMPTY_EDIT);
    expect(cellState(L, e, { newId: 1 }, "name")).toEqual({ editable: true });
    expect(cellState(L, e, { newId: 1 }, "gen")).toEqual({ editable: false, reason: "generated" });
    expect(cellState(L, e, { newId: 1 }, "ident")).toEqual({ editable: false, reason: "identityAlways" });
    expect(cellState(L, e, { newId: 9 }, "name")).toEqual({ editable: false, reason: "readOnly" });
    expect(newRowSetCell(L, e, 1, "gen", DEF)).toBe(e);
    expect(newRowSetCell(L, e, 1, "ident", T("7"))).toBe(e);
    expect(newRowSetCell(L, e, 1, "ident", NULL)).toBe(e);
    const id = newRowSetCell(L, e, 1, "ident", DEF);
    expect(toChangeSet(L, id).changes).toEqual([{ op: "insert", values: [{ column: "ident", value: DEF }] }]);
    expect(newRowSetCell(L, e, 1, "bd", T("50"))).not.toBe(e);
    const ni = loaded({ columns: COLUMNS.map((c) => (c.name === "name" ? { ...c, insertable: false } : c)) });
    expect(cellState(ni, e, { newId: 1 }, "name")).toEqual({ editable: false, reason: "notInsertable" });
    expect(newRowSetCell(ni, e, 1, "name", T("x"))).toBe(e);
    expect(cellState(loaded({ canInsert: false }), e, { newId: 1 }, "name")).toEqual({
      editable: false,
      reason: "notInsertable",
    });

    // Salt okunur kaynak: her düzenleme işlevi no-op, toChangeSet boş
    for (const ro of [
      loaded({ editable: false, readOnlyReason: "no_privilege" }),
      loaded({ relkind: "view", editable: false, readOnlyReason: "unsupported_kind" }, ROWS, false),
      loaded({ key: null, editable: false, readOnlyReason: "no_key" }),
      loaded({}, ROWS, false), // hasVersion=false (savunma)
    ]) {
      expect(cellState(ro, EMPTY_EDIT, 0, "name")).toEqual({ editable: false, reason: "readOnly" });
      expect(cellState(ro, e, { newId: 1 }, "name")).toEqual({ editable: false, reason: "readOnly" });
      expect(setCell(ro, EMPTY_EDIT, 0, "name", T("z"))).toBe(EMPTY_EDIT);
      expect(toggleDelete(ro, EMPTY_EDIT, 0)).toBe(EMPTY_EDIT);
      expect(newRowSetCell(ro, e, 1, "name", T("z"))).toBe(e);
      // Önceki (düzenlenebilir) durumdan kalan düzenleme bile gönderilmez
      const stale = toggleDelete(L, setCell(L, e, 1, "name", T("z")), 2);
      expect(toChangeSet(ro, stale)).toEqual({ changes: [], origin: [] });
    }

    // Sürümsüz / anahtarı null satır: Update/Delete üretilmez
    const odd = loaded({}, [["1", "a", null, "g", "1", "2", null], [null, "b", null, "g", "1", "2", "5"]]);
    expect(cellState(odd, EMPTY_EDIT, 0, "name")).toEqual({ editable: false, reason: "readOnly" });
    expect(setCell(odd, EMPTY_EDIT, 0, "name", T("z"))).toBe(EMPTY_EDIT);
    expect(toggleDelete(odd, EMPTY_EDIT, 1)).toBe(EMPTY_EDIT);
    expect(keyOf(odd, 1)).toEqual([]);
    const stale = toggleDelete(L, setCell(L, EMPTY_EDIT, 0, "name", T("z")), 1);
    expect(toChangeSet(odd, stale)).toEqual({ changes: [], origin: [] });
    // Kapsam dışı satır / bilinmeyen kolon
    expect(cellState(L, EMPTY_EDIT, 99, "name")).toEqual({ editable: false, reason: "readOnly" });
    expect(cellState(L, EMPTY_EDIT, 0, "nope")).toEqual({ editable: false, reason: "readOnly" });
    expect(setCell(L, EMPTY_EDIT, 99, "name", T("z"))).toBe(EMPTY_EDIT);
  });

  it("çok kolonlu anahtar sırası DataKey.columns ile", () => {
    const multi = deepFreeze({
      source: source({
        columns: [col(1, "a"), col(2, "b"), col(3, "c")],
        key: { source: "unique_index", name: "t_b_a_key", columns: ["b", "a"] },
      }),
      rows: [["A1", "B1", "C1", "7"]],
      versionIndex: 3,
      hasVersion: true,
    }) as Loaded;
    expect(keyOf(multi, 0)).toEqual(["B1", "A1"]);
    let e = setCell(multi, EMPTY_EDIT, 0, "a", T("A2"));
    e = setCell(multi, e, 0, "c", T("C2"));
    expect(toChangeSet(multi, e).changes).toEqual([
      {
        op: "update",
        key: ["B1", "A1"],
        version: "7",
        set: [
          { column: "a", value: T("A2") },
          { column: "c", value: T("C2") },
        ],
      },
    ]);
    expect(toChangeSet(multi, toggleDelete(multi, EMPTY_EDIT, 0)).changes).toEqual([
      { op: "delete", key: ["B1", "A1"], version: "7" },
    ]);
  });

  it("applyResults: UPDATE satırı returned ile yerinde (yeni sürüm), INSERT sona eklenir, DELETE'ler büyükten küçüğe çıkarılır ve indeks kayması yanlış satırı silmez; düzenleme sıfırlanır", () => {
    let e = toggleDelete(L, EMPTY_EDIT, 0);
    e = addRow(e);
    e = newRowSetCell(L, e, 1, "name", T("n1"));
    e = setCell(L, e, 2, "name", T("C"));
    e = toggleDelete(L, e, 3);
    e = addRow(e);
    e = newRowSetCell(L, e, 2, "name", T("n2"));
    const { origin } = toChangeSet(L, e);
    expect(origin.map((o) => o.kind)).toEqual(["delete", "insert", "update", "delete", "insert"]);
    const results: RowResult[] = [
      { index: 0, returned: null },
      { index: 1, returned: ["5", "n1", null, "g5", "15", "25", "200"] },
      { index: 2, returned: ["3", "C", "NULL", "g3", "13", "23", "201"] },
      { index: 3, returned: null },
      { index: 4, returned: ["6", "n2", null, "g6", "16", "26", "200"] },
    ];
    expect(resultIssues(L, origin, results)).toEqual([]);
    const next = applyResults(L, origin, results);
    expect(next.rows).toEqual([
      ["2", "b", "x", "g2", "12", "22", "101"],
      ["3", "C", "NULL", "g3", "13", "23", "201"],
      ["5", "n1", null, "g5", "15", "25", "200"],
      ["6", "n2", null, "g6", "16", "26", "200"],
    ]);
    expect(next.source).toBe(L.source);
    expect(next.versionIndex).toBe(6);
    expect(next.hasVersion).toBe(true);
    // Girdi değişmedi; düzenleme sıfırlanınca yeni satırlar temiz görünür
    expect(L.rows.length).toBe(4);
    expect(cellValueAt(next, EMPTY_EDIT, 1, "name")).toEqual({ value: { kind: "orig", value: "C" }, changed: false });
    expect(toChangeSet(next, setCell(next, EMPTY_EDIT, 1, "note", NULL)).changes).toEqual([
      { op: "update", key: ["3"], version: "201", set: [{ column: "note", value: NULL }] },
    ]);
    // Sonuçlar sıra dışı gelse de `index` ile eşlenir
    expect(applyResults(L, origin, [...results].reverse()).rows).toEqual(next.rows);
  });

  it("applyResults: returned eksik/biçimsiz → satır eklenmez/değişmez, resultIssues değişiklik indekslerini bildirir", () => {
    let e = addRow(EMPTY_EDIT);
    e = newRowSetCell(L, e, 1, "name", T("n"));
    e = setCell(L, e, 1, "name", T("B"));
    e = setCell(L, e, 2, "name", T("C"));
    const { origin } = toChangeSet(L, e);
    const results: RowResult[] = [
      { index: 0, returned: null }, // INSERT returned yok
      { index: 1, returned: ["2", "B"] }, // biçimsiz (kolon sayısı tutmuyor)
      // index 2 hiç yok
    ];
    expect(resultIssues(L, origin, results)).toEqual([0, 1, 2]);
    const next = applyResults(L, origin, results);
    expect(next.rows).toEqual(L.rows);
    expect(resultIssues(L, origin, [{ index: 0, returned: ["9", "n", null, "g", "1", "2", "3"] }, { index: 1, returned: ["2", "B", "x", "g2", "12", "22", "300"] }, { index: 2, returned: ["3", "C", "NULL", "g3", "13", "23", "300"] }])).toEqual([]);
  });

  it("modül durumsuz: aynı girdi aynı çıktı; girdi nesneleri değiştirilmez (donmuş fikstürle)", () => {
    const build = () => {
      let e = setCell(L, EMPTY_EDIT, 0, "name", T("z"));
      e = toggleDelete(L, e, 1);
      e = addRow(e);
      e = newRowSetCell(L, e, 1, "name", T("n"));
      return e;
    };
    const a = build();
    const b = build();
    expect(snap(a)).toEqual(snap(b));
    expect(toChangeSet(L, a)).toEqual(toChangeSet(L, b));
    expect(snap(EMPTY_EDIT)).toEqual({ cells: [], inserted: [], deleted: [], order: [], nextId: 1 });

    // Önceki durum sonraki işlemlerle değişmez
    const before = snap(a);
    setCell(L, a, 0, "note", T("q"));
    revertCell(a, 0, "name");
    toggleDelete(L, a, 1);
    toggleDelete(L, a, 2);
    removeNewRow(a, 1);
    addRow(a);
    newRowSetCell(L, a, 1, "note", T("m"));
    toChangeSet(L, a);
    const { origin } = toChangeSet(L, a);
    applyResults(L, origin, [{ index: 0, returned: ["1", "z", null, "g1", "11", "21", "300"] }]);
    expect(snap(a)).toEqual(before);
    expect(Object.isFrozen(L.rows[0])).toBe(true);
    expect(L.rows[0]).toEqual(ROWS[0]);
  });
});
