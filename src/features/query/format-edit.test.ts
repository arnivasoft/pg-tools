import { history, undo, undoDepth } from "@codemirror/commands";
import { EditorState, type Transaction } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { formatRange, formatReplacement, formatTransaction, mapCursor, safeFormatSelection } from "./format-edit";

describe("formatRange", () => {
  it("boş seçimde tüm belge, aksi halde seçim (ters seçim dahil)", () => {
    expect(formatRange(20, 5, 5)).toEqual({ from: 0, to: 20, whole: true });
    expect(formatRange(20, 9, 3)).toEqual({ from: 3, to: 9, whole: false });
  });
});

describe("formatReplacement", () => {
  it("değişiklik yoksa null", () => {
    expect(formatReplacement("SELECT 1", "SELECT 1", true)).toBeNull();
  });
  it("tüm belge olduğu gibi", () => {
    expect(formatReplacement("select 1\n", "SELECT 1\n", true)).toBe("SELECT 1\n");
  });
  it("seçimde baştaki/sondaki boşluk korunur", () => {
    expect(formatReplacement("  select a,b from t \n", "SELECT a, b\nFROM t\n", false)).toBe("  SELECT a, b\nFROM t \n");
  });
});

describe("mapCursor", () => {
  const original = "select a,b from t where x=1";
  const formatted = "SELECT a, b\nFROM t\nWHERE x=1";
  it("belirtecin önündeki imleç aynı belirtecin önüne", () => {
    const at = original.indexOf("from");
    expect(formatted.slice(mapCursor(original, formatted, at))).toMatch(/^FROM t/);
    const b = original.indexOf("b");
    expect(formatted.slice(mapCursor(original, formatted, b))).toMatch(/^b\n/);
  });
  it("boşluktaki imleç önceki belirtecin ardına", () => {
    const at = original.indexOf(" from"); // "b|" sonrası boşluk
    expect(formatted.slice(0, mapCursor(original, formatted, at))).toBe("SELECT a, b");
  });
  it("belirteç ortası korunur; uçlar", () => {
    const at = original.indexOf("ere"); // wh|ere
    expect(formatted.slice(mapCursor(original, formatted, at))).toMatch(/^ERE/);
    expect(mapCursor(original, formatted, 0)).toBe(0);
    expect(mapCursor(original, formatted, original.length)).toBe(formatted.length);
  });
  it("virgül başta düzeni ve emoji", () => {
    const o = "select '😀',b from t";
    const f = "SELECT '😀'\n, b\nFROM t";
    expect(f.slice(mapCursor(o, f, o.indexOf("b")))).toMatch(/^b\n/);
  });
  it("boşluk dışı dizi farklıysa orantılı yedek", () => {
    expect(mapCursor("abcd", "xy", 2)).toBe(1);
    expect(mapCursor("", "xy", 0)).toBe(2);
  });
});

describe("formatTransaction", () => {
  const run = (state: EditorState, cmd: (t: { state: EditorState; dispatch: (tr: Transaction) => void }) => boolean) => {
    let next = state;
    cmd({ state, dispatch: (tr) => (next = tr.state) });
    return next;
  };
  it("tek geri alma adımı; yazmayla birleşmez; imleç eşlenir", () => {
    let state = EditorState.create({ doc: "", extensions: [history()] });
    state = state.update({ changes: { from: 0, insert: "select a,b from t" }, userEvent: "input.type" }).state;
    state = state.update({ selection: { anchor: "select a,b ".length } }).state; // "from"un önü
    const doc = state.doc.toString();
    const spec = formatTransaction(state, doc, 0, doc.length, "SELECT a, b\nFROM t");
    expect(spec).not.toBeNull();
    const formatted = state.update(spec!).state;
    expect(formatted.doc.toString()).toBe("SELECT a, b\nFROM t");
    expect(formatted.doc.toString().slice(formatted.selection.main.head)).toBe("FROM t");
    expect(undoDepth(formatted)).toBe(2);
    const undone = run(formatted, undo);
    expect(undone.doc.toString()).toBe("select a,b from t");
  });
  it("belge değiştiyse ya da salt okunursa işlem yok", () => {
    const state = EditorState.create({ doc: "select 1" });
    expect(formatTransaction(state, "select 2", 0, 8, "SELECT 2")).toBeNull();
    const ro = EditorState.create({ doc: "select 1", extensions: EditorState.readOnly.of(true) });
    expect(formatTransaction(ro, "select 1", 0, 8, "SELECT 1")).toBeNull();
  });
  it("seçim biçimlendirmesi yalnız aralığı değiştirir", () => {
    const doc = "select 1;\nselect a,b from t;\n";
    const from = doc.indexOf("select a");
    const to = doc.lastIndexOf(";");
    const state = EditorState.create({ doc, selection: { anchor: from, head: to } });
    const insert = formatReplacement(doc.slice(from, to), "SELECT a, b\nFROM t", false)!;
    const next = state.update(formatTransaction(state, doc, from, to, insert)!).state;
    expect(next.doc.toString()).toBe("select 1;\nSELECT a, b\nFROM t;\n");
  });
});

describe("safeFormatSelection", () => {
  const sel = (doc: string, part: string, occurrence = 0) => {
    let from = -1;
    for (let i = 0; i <= occurrence; i++) from = doc.indexOf(part, from + 1);
    return [from, from + part.length] as const;
  };
  it("kodda başlayıp biten seçim aynen kalır", () => {
    const doc = "select 1;\nselect a,b from t;\n";
    const [f, t] = sel(doc, "select a,b from t");
    expect(safeFormatSelection(doc, f, t)).toEqual({ from: f, to: t, snapped: false });
  });
  it("dize içindeki seçim çevreleyen ifadeye genişler (dize kod sanılmaz)", () => {
    const doc = "select 1;\ninsert into t values ('select  a  from b');\nselect 2;";
    const [f, t] = sel(doc, "select  a  from b");
    const r = safeFormatSelection(doc, f, t)!;
    expect(doc.slice(r.from, r.to)).toBe("\ninsert into t values ('select  a  from b')");
    expect(r.snapped).toBe(true);
  });
  it("çok satırlı dizenin içinde üç tıkla seçilmiş satır ifadeye genişler", () => {
    const doc = "select 'first\n  two  spaces  here\nlast' as x;\nselect 3;";
    const [f, t] = sel(doc, "  two  spaces  here\n");
    const r = safeFormatSelection(doc, f, t)!;
    expect(doc.slice(r.from, r.to)).toBe("select 'first\n  two  spaces  here\nlast' as x");
  });
  it("tırnaklı ad, yorum ve dolar gövdesi içinde başlayan seçim genişler", () => {
    const quoted = 'select * from "My  Table" t;';
    const [f1, t1] = sel(quoted, "  Table");
    expect(safeFormatSelection(quoted, f1, t1)).toMatchObject({ from: 0, to: quoted.length - 1 });
    const comment = "select 1 /* select  a  from b */ + 2;";
    const [f2, t2] = sel(comment, "select  a");
    expect(safeFormatSelection(comment, f2, t2)).toMatchObject({ from: 0, to: comment.length - 1 });
    const dollar = "do $$ begin perform  1; end $$;";
    const [f3] = sel(dollar, "begin");
    expect(safeFormatSelection(dollar, f3, f3 + 5)).toMatchObject({ from: 0, to: dollar.length - 1 });
  });
  it("belirteç ortasında başlayan ya da biten seçim genişler", () => {
    const doc = "select customer from orders;";
    const [f] = sel(doc, "tomer");
    expect(safeFormatSelection(doc, f, doc.indexOf(" from"))).toEqual({ from: 0, to: doc.indexOf(" from"), snapped: true });
    const cast = "select x::int from t;";
    expect(safeFormatSelection(cast, cast.indexOf("::") + 1, cast.length - 1)).toMatchObject({ from: 0 });
    const prefixed = "select E'a\\'b' from t;";
    expect(safeFormatSelection(prefixed, prefixed.indexOf("'"), prefixed.length - 1)).toMatchObject({ from: 0 });
    const ends = "select a from orders where x = 1;";
    expect(safeFormatSelection(ends, 0, ends.indexOf("ders"))).toMatchObject({ from: 0, to: ends.length - 1 });
  });
  it("dize içinde biten seçim ifade sonuna genişler; belge uçları her zaman güvenli", () => {
    const doc = "select 'a  b' as x; select 2";
    expect(safeFormatSelection(doc, 0, doc.indexOf("  b"))).toEqual({ from: 0, to: doc.indexOf(";"), snapped: true });
    const tail = "select 1 -- son";
    expect(safeFormatSelection(tail, 0, tail.length)).toEqual({ from: 0, to: tail.length, snapped: false });
  });
  it("kapanmamış dize belge sonuna kadar sürer: seçim belge sonuna genişler", () => {
    const doc = "select 1; select 'open  text\nmore";
    const [f, t] = sel(doc, "open");
    const r = safeFormatSelection(doc, f, t);
    expect(r).toEqual({ from: doc.indexOf(" select 'open"), to: doc.length, snapped: true });
  });
});
