import { insertNewlineAndIndent } from "@codemirror/commands";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { bodyEditingExtensions, copyIndentColumn, sqlLookExtensions } from "./codemirror-setup";

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
