import { insertNewlineAndIndent } from "@codemirror/commands";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { bodyEditingExtensions, bodyIndentColumn, copyIndentColumn, sqlLookExtensions } from "./codemirror-setup";

/** Görünümsüz Enter: imleç `pos`'ta, sonuç metni. */
function enter(doc: string, pos: number): string {
  let out = doc;
  const state = EditorState.create({ doc, selection: { anchor: pos }, extensions: bodyEditingExtensions("  ") });
  insertNewlineAndIndent({ state, dispatch: (tr) => (out = tr.state.doc.toString()) });
  return out;
}

describe("codemirror-setup: gövde girintisi (önceki dolu satırı kopyala)", () => {
  it("copyIndentColumn: en yakın boş olmayan satırın girinti sütunu; yoksa 0", () => {
    expect(copyIndentColumn(["    x := 1;"], 4)).toBe(4);
    expect(copyIndentColumn(["", "   ", "  BEGIN"], 4)).toBe(2);
    expect(copyIndentColumn(["\tRETURN 1;"], 4)).toBe(4);
    expect(copyIndentColumn(["", ""], 4)).toBe(0);
    expect(copyIndentColumn([], 4)).toBe(0);
  });

  it("Enter önceki satırın girintisini sürdürür (PL/pgSQL bloğu modellenmez)", () => {
    expect(enter("BEGIN\n  x := 1;", 15)).toBe("BEGIN\n  x := 1;\n  ");
    expect(enter("BEGIN", 5)).toBe("BEGIN\n");
    // Boş satır atlanır: üstteki dolu satırın girintisi
    expect(enter("    a;\n", 7)).toBe("    a;\n\n    ");
  });
});

describe("codemirror-setup: girinti servisi SQL dilinin girintisinden önce gelir", () => {
  it("SQL vurgusu açıkken de Enter önceki dolu satırın girintisini kopyalar", () => {
    const doc = "BEGIN\n    IF x THEN\n\n";
    let out = doc;
    const state = EditorState.create({
      doc,
      selection: { anchor: doc.length },
      extensions: [bodyEditingExtensions("  "), sqlLookExtensions()],
    });
    insertNewlineAndIndent({ state, dispatch: (tr) => (out = tr.state.doc.toString()) });
    expect(out).toBe(`${doc}\n    `);
  });
});

describe("codemirror-setup: Enter bulunduğu satırın kendi girintisini korur (her iki kip)", () => {
  /** `sql`: SQL dili de açık (vurgu kipi); değilse düz metin kipi. */
  function enterIn(doc: string, pos: number, sqlMode: boolean): string {
    let out = doc;
    const state = EditorState.create({
      doc,
      selection: { anchor: pos },
      extensions: [bodyEditingExtensions("  "), sqlMode ? sqlLookExtensions() : []],
    });
    insertNewlineAndIndent({ state, dispatch: (tr) => (out = tr.state.doc.toString()) });
    return out;
  }

  for (const sqlMode of [false, true]) {
    const kip = sqlMode ? "sql" : "düz";
    it(`${kip}: ifadenin önünde Enter girintiyi düşürmez (plpython)`, () => {
      const doc = "def f():\n    if x:\n        return 1";
      // `return` 8 sütunda kalır. Önceki satırda boşluk kalması CodeMirror davranışıdır (`newlineAndIndent`'in yalnız-boşluk
      // denetimi satırı belge ofsetiyle dilimler; ilk satırlar dışında imlecin önündeki boşluğu silmez).
      expect(enterIn(doc, 27, sqlMode)).toBe("def f():\n    if x:\n        \n        return 1");
    });
    it(`${kip}: ilk satırda baştaki boşluğun sonunda Enter metnin girintisini korur`, () => {
      expect(enterIn("  x", 2, sqlMode)).toBe("\n  x");
      expect(enterIn("  x", 3, sqlMode)).toBe("  x\n  ");
    });
    it(`${kip}: yalnız boşluktan oluşan satırın sonunda Enter o girintiyi sürdürür`, () => {
      expect(enterIn("BEGIN\n    ", 10, sqlMode)).toBe("BEGIN\n\n    ");
    });
    it(`${kip}: satır başında Enter satırı girintisiyle aşağı taşır`, () => {
      expect(enterIn("BEGIN\n    x;", 6, sqlMode)).toBe("BEGIN\n\n    x;");
    });
    it(`${kip}: boş satırda Enter üstteki dolu satırın girintisini kopyalar`, () => {
      expect(enterIn("    a;\n", 7, sqlMode)).toBe("    a;\n\n    ");
    });
  }

  it("bodyIndentColumn: satır boş değilse kendi baştaki boşluğu, boşsa önceki dolu satır", () => {
    expect(bodyIndentColumn("        return 1", ["    if x:"], 4)).toBe(8);
    expect(bodyIndentColumn("    ", ["BEGIN"], 4)).toBe(4);
    expect(bodyIndentColumn("x", ["    a"], 4)).toBe(0);
    expect(bodyIndentColumn("", ["", "  a"], 4)).toBe(2);
    expect(bodyIndentColumn("", [], 4)).toBe(0);
  });
});
