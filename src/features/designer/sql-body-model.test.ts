import { EditorSelection, EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { minimalChange, separatorOf } from "./sql-body-model";

/** Gerçek CodeMirror durumuna uygular: sonuç metni ve eşlenen imleç. */
function apply(a: string, b: string, cursor = 0) {
  const sep = separatorOf(a);
  const st = EditorState.create({ doc: a, selection: EditorSelection.cursor(cursor), extensions: [EditorState.lineSeparator.of(sep)] });
  const ch = minimalChange(st.sliceDoc(), b, sep);
  const next = ch ? st.update({ changes: ch }).state : st;
  return { text: next.sliceDoc(), cursor: next.selection.main.head };
}

describe("sql-body-model", () => {
  it("separatorOf: CRLF varsa CRLF, aksi halde LF", () => {
    expect(separatorOf("a\r\nb")).toBe("\r\n");
    expect(separatorOf("a\nb")).toBe("\n");
    expect(separatorOf("a\rb")).toBe("\n");
    expect(separatorOf("")).toBe("\n");
  });

  it("minimalChange: eşitse null; yalnızca farklı orta bölüm", () => {
    expect(minimalChange("abc", "abc")).toBeNull();
    expect(minimalChange("BEGIN x END", "BEGIN yy END")).toEqual({ from: 6, to: 7, insert: "yy" });
    expect(minimalChange("", "abc")).toEqual({ from: 0, to: 0, insert: "abc" });
    expect(minimalChange("abc", "")).toEqual({ from: 0, to: 3, insert: "" });
  });

  it("LF: uygulanan değişiklik hedef metni verir; değişiklikten sonraki imleç eşlenir", () => {
    const a = "BEGIN\n  RETURN NEW;\nEND;";
    const b = "BEGIN\n  PERFORM 1;\n  RETURN NEW;\nEND;";
    const r = apply(a, b, a.length);
    expect(r.text).toBe(b);
    expect(r.cursor).toBe(b.length);
    // Değişiklikten önceki imleç yerinde kalır
    expect(apply(a, b, 2).cursor).toBe(2);
  });

  it("CRLF korunur: ayırıcı CRLF, konumlar belge konumu (satır sonu tek konum)", () => {
    const a = "BEGIN\r\n  RETURN NEW;\r\nEND;";
    const b = "BEGIN\r\n  RETURN OLD;\r\nEND;";
    expect(apply(a, b).text).toBe(b);
    const c = "BEGIN\r\n  PERFORM 1;\r\n  RETURN NEW;\r\nEND;";
    const docLen = EditorState.create({ doc: a, extensions: [EditorState.lineSeparator.of("\r\n")] }).doc.length;
    const r = apply(a, c, docLen);
    expect(r.text).toBe(c);
    expect(r.cursor).toBe(EditorState.create({ doc: c, extensions: [EditorState.lineSeparator.of("\r\n")] }).doc.length);
  });

  it("CRLF çifti bölünmez (satır sonu ekleme/silme)", () => {
    const cases: Array<[string, string]> = [
      ["a\r\nb", "a\r\n\r\nb"],
      ["a\r\n\r\nb", "a\r\nb"],
      ["a\r\nb\r\n", "a\r\nb"],
      ["a\r\nb", "a\r\nb\r\n"],
      ["x\r\ny", "x\r\nz\r\ny"],
      ["a\r\nb", "ab\r\nb"],
    ];
    for (const [a, b] of cases) expect(apply(a, b).text).toBe(b);
  });

  it("LF belgede tek başına \\r sıradan karakter kalır", () => {
    expect(apply("a\rb\nc", "a\rB\nc").text).toBe("a\rB\nc");
  });
});
