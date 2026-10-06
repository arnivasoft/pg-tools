import { describe, expect, it } from "vitest";
import { errorRange, scalarToUtf16, scopeFromEditor, stmtText, utf16ToScalar } from "./offsets";

describe("utf16ToScalar / scalarToUtf16", () => {
  const cases: [string, string, number, number][] = [
    ["ascii", "select 1", 4, 4],
    ["türkçe", "çğü x", 3, 3],
    ["emoji sonrası", "a😀b", 3, 2],
    ["birleşik karakter", "éx", 2, 2],
    ["metin sonu", "a😀", 3, 2],
  ];
  for (const [name, text, u16, sc] of cases) {
    it(name, () => {
      expect(utf16ToScalar(text, u16)).toBe(sc);
      expect(scalarToUtf16(text, sc)).toBe(u16);
    });
  }
  it("vekil ortası öncekine yuvarlanır", () => {
    expect(utf16ToScalar("a😀b", 2)).toBe(1);
  });
  it("sınır dışı kırpılır", () => {
    expect(utf16ToScalar("a😀", -5)).toBe(0);
    expect(utf16ToScalar("a😀", 99)).toBe(2);
    expect(scalarToUtf16("a😀", -1)).toBe(0);
    expect(scalarToUtf16("a😀", 99)).toBe(3);
  });
  it("gidiş-dönüş (sabit tohum, kelime sınırlarında)", () => {
    let seed = 12345;
    const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
    const alphabet = ["a", "ç", "😀", "é", "é", " ", "𝒳", "1"];
    for (let n = 0; n < 200; n++) {
      let text = "";
      const len = Math.floor(rnd() * 20);
      for (let i = 0; i < len; i++) text += alphabet[Math.floor(rnd() * alphabet.length)];
      const count = Array.from(text).length;
      for (let s = 0; s <= count; s++) {
        const u = scalarToUtf16(text, s);
        expect(utf16ToScalar(text, u)).toBe(s);
        expect(scalarToUtf16(text, utf16ToScalar(text, u))).toBe(u);
      }
    }
  });
});

describe("scopeFromEditor", () => {
  const text = "😀 select 1; select 2";
  it("all", () => expect(scopeFromEditor(text, 0, 0, 0, "all")).toEqual({ type: "all" }));
  it("emoji sonrası seçim scalar aralığı", () => {
    // "😀 " = 3 UTF-16, 2 scalar; "select 1" UTF-16 3..11
    expect(scopeFromEditor(text, 3, 11, 11, "run")).toEqual({ type: "selection", start: 2, end: 10 });
  });
  it("ters seçim sıralanır", () => {
    expect(scopeFromEditor(text, 11, 3, 3, "run")).toEqual({ type: "selection", start: 2, end: 10 });
  });
  it("boş seçim imleç", () => {
    expect(scopeFromEditor(text, 5, 5, 5, "run")).toEqual({ type: "atCursor", offset: 4 });
  });
});

describe("errorRange", () => {
  it("emoji sonrası x", () => {
    expect(errorRange("select '😀' || x", { charOffset: 0, position: 15 })).toEqual({ from: 15, to: 16 });
  });
  it("kelime sonuna kadar", () => {
    expect(errorRange("select foo, 1", { charOffset: 0, position: 8 })).toEqual({ from: 7, to: 10 });
  });
  it("charOffset ile betik içi konum", () => {
    const text = "😀; select bad_col from t";
    // ifade scalar 3'te başlar; position 8 -> bad_col (scalar 10, UTF-16 11)
    expect(errorRange(text, { charOffset: 3, position: 8 })).toEqual({ from: 11, to: 18 });
  });
  it("noktalama tek karakter", () => {
    expect(errorRange("a,b", { charOffset: 0, position: 2 })).toEqual({ from: 1, to: 2 });
  });
  it("position null", () => expect(errorRange("x", { charOffset: 0, position: null })).toBeNull());
  it("metin sonu kırpılır", () => {
    expect(errorRange("ab", { charOffset: 0, position: 99 })).toEqual({ from: 2, to: 2 });
  });
  describe("nitelikli ad", () => {
    const at = (text: string, needle: string) => ({ charOffset: 0, position: [...text.slice(0, text.indexOf(needle))].length + 1 });
    const mark = (text: string, needle: string) => {
      const r = errorRange(text, at(text, needle))!;
      return text.slice(r.from, r.to);
    };
    it("şema.tablo tamamı", () => expect(mark("select * from d001.vstokhar1", "d001")).toBe("d001.vstokhar1"));
    it("tırnaklı parçalar ve \"\" kaçışı", () => {
      expect(mark('select * from "Weird"."Name"', '"Weird"')).toBe('"Weird"."Name"');
      expect(mark('select * from "a""b".c where 1', '"a""b"')).toBe('"a""b".c');
    });
    it("takma ad dahil edilmez", () => expect(mark("select * from app.orders o", "app")).toBe("app.orders"));
    it("fonksiyon çağrısında yalnızca ad", () => expect(mark("select public.f(1)", "public")).toBe("public.f"));
    it("$ ad içinde", () => expect(mark("select * from s.t$1 x", "s.")).toBe("s.t$1"));
    it("t.* yıldızı almaz", () => expect(mark("select t.* from t", "t.")).toBe("t"));
    it("nokta çevresinde boşluk", () => expect(mark("select * from app . orders o", "app")).toBe("app . orders"));
    it("emoji sonrası (UTF-16)", () => {
      const text = "select '😀', x from public.nope_tbl";
      const r = errorRange(text, at(text, "public"))!;
      expect(r).toEqual({ from: 20, to: 35 });
      expect(text.slice(r.from, r.to)).toBe("public.nope_tbl");
    });
    it("metin sonundaki ad", () => expect(mark("select * from s.t", "s.")).toBe("s.t"));
    it("kapanmamış tırnak tek karakter", () => expect(mark('select "abc', '"')).toBe('"'));
    it("işleç tek karakter", () => expect(mark("select 1 ++ x", "++")).toBe("+"));
    it("sayı eski davranış", () => expect(mark("select 12.5x", "12")).toBe("12"));
  });
});

describe("stmtText", () => {
  it("emoji sonrası ifade", () => {
    expect(stmtText("😀; select 1; x", { charOffset: 3, charLen: 8 })).toBe("select 1");
  });
});
