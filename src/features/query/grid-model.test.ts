import { describe, expect, it } from "vitest";
import type { Column } from "../../ipc/bindings/Column";
import {
  CELL_PANEL_MAX, reindentJson, cellPanelText, displayCell, extendTo, initialWidths, inRect, isNumericType, moveFocus,
  selectAll, selectCell, selectColumn, selectRow, selectionToTsv, selRect,
} from "./grid-model";

const col = (name: string, typeName = "text", typeDisplay = typeName): Column => ({ name, typeName, typeOid: 0, typeDisplay });
const m = { header: (s: string) => s.length * 8, type: (s: string) => s.length * 8, cell: (s: string) => s.length * 8 };

describe("displayCell", () => {
  it("NULL ile boş metni ayırır", () => {
    expect(displayCell(null)).toEqual({ text: "", null: true, truncated: false });
    expect(displayCell("")).toEqual({ text: "", null: false, truncated: false });
  });
  it("satır sonlarını ⏎ yapar", () => {
    expect(displayCell("a\r\nb\nc\rd").text).toBe("a⏎b⏎c⏎d");
  });
  it("1000 karakterde kırpar", () => {
    const r = displayCell("x".repeat(1500));
    expect(r.truncated).toBe(true);
    expect(r.text).toBe("x".repeat(1000) + "…");
    expect(displayCell("x".repeat(1000)).truncated).toBe(false);
  });
  it("vekil çiftini bölmez", () => {
    const r = displayCell("x".repeat(999) + "😀");
    expect(r.text).toBe("x".repeat(999) + "…");
  });
});

describe("selectionToTsv", () => {
  const cols = [col("a"), col("b\tx")];
  const rows = [["1", null], ["a\tb", 'q"r'], ["l1\nl2", ""]];
  it("kaçışlar ve NULL", () => {
    const tsv = selectionToTsv(rows, cols, { r0: 0, r1: 2, c0: 0, c1: 1 }, { headers: false });
    expect(tsv).toBe('1\t\n"a\tb"\t"q""r"\n"l1\nl2"\t');
  });
  it("başlıklar ve alt dikdörtgen", () => {
    expect(selectionToTsv(rows, cols, { r0: 0, r1: 0, c0: 0, c1: 1 }, { headers: true })).toBe('a\t"b\tx"\n1\t');
    expect(selectionToTsv(rows, cols, { r0: 1, r1: 1, c0: 1, c1: 1 }, { headers: false })).toBe('"q""r"');
  });
});

describe("initialWidths", () => {
  it("sınırlar 60..480", () => {
    const w = initialWidths([col("a"), col("b"), col("c")], [["x", "y".repeat(500), null]], m);
    expect(w[0]).toBe(60);
    expect(w[1]).toBe(480);
    expect(w[2]).toBe(72); // "[NULL]" 48 + 24 dolgu
  });
  it("yalnızca ilk 200 satırı örnekler", () => {
    const rows = Array.from({ length: 300 }, (_, i) => [i === 250 ? "z".repeat(40) : "a"]);
    expect(initialWidths([col("a")], rows, m)[0]).toBe(60);
  });
  it("kısa içerikte başlık adı ve tür etiketi kırpılmaz (kendi yazı tipleriyle ölçülür)", () => {
    // Başlık yarı kalın sans (9 px/karakter), tür küçük sans (6), hücre eş aralıklı (7).
    const fonts = { header: (s: string) => s.length * 9, type: (s: string) => s.length * 6, cell: (s: string) => s.length * 7 };
    const longName = col("musteri_hesap_numarasi", "int8", "bigint");
    const longType = col("id", "timestamptz", "timestamp with time zone");
    const w = initialWidths([longName, longType], [["1", "2"]], fonts);
    expect(w[0]).toBe(22 * 9 + 24); // ad, hücre ("1") ve türden geniş
    expect(w[1]).toBe(24 * 6 + 24); // tür etiketi addan ve hücreden geniş
  });
  it("uzun içerik yine 480'de durur", () => {
    const fonts = { header: (s: string) => s.length * 9, type: (s: string) => s.length * 6, cell: (s: string) => s.length * 7 };
    expect(initialWidths([col("a", "varchar", "character varying(50)")], [["x".repeat(200)]], fonts)[0]).toBe(480);
  });
});

describe("isNumericType", () => {
  it("sayısal türler", () => {
    for (const t of ["int2", "int4", "int8", "numeric", "float4", "float8", "oid", "money"]) expect(isNumericType(t)).toBe(true);
    for (const t of ["text", "uuid", "bool", "jsonb"]) expect(isNumericType(t)).toBe(false);
  });
});

describe("seçim", () => {
  it("hücre/satır/sütun/tümü", () => {
    expect(selRect(selectCell(2, 1))).toEqual({ r0: 2, r1: 2, c0: 1, c1: 1 });
    expect(selRect(selectRow(3, 4))).toEqual({ r0: 3, r1: 3, c0: 0, c1: 3 });
    expect(selRect(selectColumn(2, 10))).toEqual({ r0: 0, r1: 9, c0: 2, c1: 2 });
    expect(selRect(selectAll(5, 3))).toEqual({ r0: 0, r1: 4, c0: 0, c1: 2 });
  });
  it("Shift ile genişletme ters yönde de çalışır", () => {
    const s = extendTo(selectCell(5, 5), { r: 2, c: 7 });
    expect(selRect(s)).toEqual({ r0: 2, r1: 5, c0: 5, c1: 7 });
    expect(inRect(selRect(s), 3, 6)).toBe(true);
    expect(inRect(selRect(s), 6, 6)).toBe(false);
  });
  it("gezinme kıstırılır", () => {
    expect(moveFocus({ r: 0, c: 0 }, "ArrowUp", 10, 3, 5)).toEqual({ r: 0, c: 0 });
    expect(moveFocus({ r: 8, c: 2 }, "PageDown", 10, 3, 5)).toEqual({ r: 9, c: 2 });
    expect(moveFocus({ r: 4, c: 1 }, "End", 10, 3, 5)).toEqual({ r: 4, c: 2 });
    expect(moveFocus({ r: 4, c: 1 }, "Home", 10, 3, 5, true)).toEqual({ r: 0, c: 0 });
    expect(moveFocus({ r: 4, c: 1 }, "ArrowRight", 0, 0, 5)).toEqual({ r: 0, c: 0 });
  });
});

describe("cellPanelText", () => {
  it("json türünü girintiler", () => {
    expect(cellPanelText('{"a":1,"b":[1,2],"c":{},"d":[]}', "jsonb").text).toBe(
      '{\n  "a": 1,\n  "b": [\n    1,\n    2\n  ],\n  "c": {},\n  "d": []\n}',
    );
  });
  it("kayıpsız: büyük tamsayı, 1.10, 1e400, yinelenen anahtar", () => {
    const out = cellPanelText('{"a":12345678901234567890,"b":1.10,"c":1e400,"k":1,"k":2}', "json").text;
    for (const frag of ["12345678901234567890", "1.10", "1e400", '"k": 1', '"k": 2']) expect(out).toContain(frag);
  });
  it("metin içindeki parantez ve kaçışlı tırnak korunur", () => {
    expect(reindentJson('{"a":"x{[,]}\\"y"}')).toBe('{\n  "a": "x{[,]}\\"y"\n}');
  });
  it("geçersiz JSON ham kalır; json olmayan sütun asla biçimlenmez", () => {
    expect(cellPanelText("{bozuk", "jsonb").text).toBe("{bozuk");
    expect(cellPanelText('{"a":1}', "text").text).toBe('{"a":1}');
    expect(cellPanelText("[1,2]", "text").text).toBe("[1,2]");
  });
  it("1 MB üstünü kırpar", () => {
    const r = cellPanelText("x".repeat(CELL_PANEL_MAX + 5), "text");
    expect(r.clipped).toBe(true);
    expect(r.text.length).toBe(CELL_PANEL_MAX);
  });
  it("kırpma vekil çiftini bölmez", () => {
    const r = cellPanelText("x".repeat(CELL_PANEL_MAX - 1) + "😀", "text");
    expect(r.clipped).toBe(true);
    expect(r.text.length).toBe(CELL_PANEL_MAX - 1);
  });
});
