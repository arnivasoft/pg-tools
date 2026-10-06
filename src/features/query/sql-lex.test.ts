import { describe, expect, it } from "vitest";
import cases from "../../../crates/pgt-query/tests/fixtures/split-cases.json";
import { statementBounds } from "./completion";
import { lexSql, type SqlTokKind } from "./sql-lex";

const kinds = (s: string): [SqlTokKind, string][] => lexSql(s).map((t) => [t.kind, s.slice(t.start, t.end)]);

describe("lexSql (lex.rs karşılığı)", () => {
  it("temel belirteçler", () => {
    expect(kinds("select 'a;b'")).toEqual([["word", "select"], ["str", "'a;b'"]]);
    expect(kinds("select $a$x$$y$a$")).toEqual([["word", "select"], ["dollar", "$a$x$$y$a$"]]);
    expect(kinds("select a$b$ from t")).toEqual([["word", "select"], ["word", "a$b$"], ["word", "from"], ["word", "t"]]);
    expect(kinds("select $1")).toEqual([["word", "select"], ["punct", "$"], ["word", "1"]]);
    expect(kinds("'it''s'")).toEqual([["str", "'it''s'"]]);
    expect(kinds("/* a /* b */ c */ x")).toEqual([["comment", "/* a /* b */ c */"], ["word", "x"]]);
  });
  it("E/e öneki yalnızca bitişik tek karakterlik sözcükte; içinde ters bölü kaçıştır", () => {
    expect(kinds("E'\\''")).toEqual([["str", "E'\\''"]]);
    expect(kinds("e'a\\'b'")).toEqual([["str", "e'a\\'b'"]]);
    expect(kinds("E'\\\\'x")).toEqual([["str", "E'\\\\'"], ["word", "x"]]);
    expect(kinds("E'\\\\\\''")).toEqual([["str", "E'\\\\\\''"]]);
    expect(kinds("name'a\\'")).toEqual([["word", "name"], ["str", "'a\\'"]]);
    expect(kinds("é'a'")).toEqual([["word", "é"], ["str", "'a'"]]);
    expect(kinds("😀e'a\\'")).toEqual([["word", "😀e"], ["str", "'a\\'"]]);
    expect(kinds("E 'a\\'")).toEqual([["word", "E"], ["str", "'a\\'"]]);
    expect(kinds("x=E'a'")).toEqual([["word", "x"], ["punct", "="], ["str", "E'a'"]]);
    expect(kinds("$e'\\''")).toEqual([["punct", "$"], ["str", "e'\\''"]]);
    expect(kinds("U&'\\'")).toEqual([["word", "U"], ["punct", "&"], ["str", "'\\'"]]);
    expect(kinds("B'1' X'f' N'\\'")).toEqual([["word", "B"], ["str", "'1'"], ["word", "X"], ["str", "'f'"], ["word", "N"], ["str", "'\\'"]]);
  });
  it("kapanmamış belirteçler metin sonuna kadar sürer ve kapanmamış işaretlenir", () => {
    for (const s of ["'abc", "/* abc", "$$abc", '"abc', "E'abc\\", "E'abc\\😀", "E'\\'", "$t$ab", "/* a /* b */"]) {
      const t = lexSql(s);
      expect(t[t.length - 1]).toMatchObject({ end: s.length, closed: false });
    }
    expect(lexSql("-- x")[0]!.closed).toBe(false);
    expect(lexSql("-- x\n")[0]!.closed).toBe(true);
  });
  it("rastgele girdide aralıklar geçerli ve sıralı", () => {
    const alphabet = [..."'\"$-/*;()\\eE_ \n\t1aBx=,.çğüİıé€😀"];
    let seed = 0x9e3779b9;
    const rnd = () => {
      seed ^= seed << 13;
      seed ^= seed >>> 17;
      seed ^= seed << 5;
      return seed >>> 0;
    };
    for (let k = 0; k < 5000; k++) {
      const n = rnd() % 40;
      let s = "";
      for (let i = 0; i < n; i++) s += alphabet[rnd() % alphabet.length];
      let prev = 0;
      for (const t of lexSql(s)) {
        expect(t.start < t.end && t.end <= s.length && t.start >= prev).toBe(true);
        prev = t.end;
      }
      const b = statementBounds(s, rnd() % (s.length + 1));
      expect(0 <= b.from && b.from <= b.to && b.to <= s.length).toBe(true);
    }
  });
});

/**
 * `statementBounds` ile metindeki ifadeler. Arka uç ifadesi ilk yorum-dışı belirteçte başlar: baştaki yorumlar atlanır
 * (kapanmamış iç içe yorum dahil), yalnız boşluk/yorum içeren parçalar ifade sayılmaz.
 */
function frontEndStatements(sql: string): string[] {
  const out: string[] = [];
  let pos = 0;
  for (;;) {
    const b = statementBounds(sql, pos);
    const seg = sql.slice(b.from, b.to);
    const first = lexSql(seg).find((t) => t.kind !== "comment");
    if (first) out.push(seg.slice(first.start).trimEnd());
    if (b.to >= sql.length) break;
    pos = b.to + 1;
  }
  return out;
}

describe("ortak bölme tablosu (crates/pgt-query/tests/fixtures/split-cases.json)", () => {
  it("tablo dolu", () => expect(cases.length).toBeGreaterThan(20));
  for (const c of cases) {
    it(JSON.stringify(c.sql), () => {
      expect(frontEndStatements(c.sql)).toEqual(c.stmts.map((s) => s.trimEnd()));
    });
  }
});
