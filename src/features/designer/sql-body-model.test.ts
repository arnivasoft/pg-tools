import { describe, expect, it } from "vitest";
import { INDENT, applyDisplayEdit, bodyLanguageMode, displayDiff, displayText, separatorOf } from "./sql-body-model";

/** Görüntü metnine (CodeMirror belgesi, yalnız `\n`) `[from, to)` → `insert` değişikliği. */
const edit = (doc: string, from: number, to: number, insert: string) => doc.slice(0, from) + insert + doc.slice(to);


describe("sql-body-model: satır sonu", () => {
  it("separatorOf: CRLF varsa CRLF, aksi halde LF", () => {
    expect(separatorOf("a\r\nb")).toBe("\r\n");
    expect(separatorOf("a\nb")).toBe("\n");
    expect(separatorOf("a\rb")).toBe("\n");
    expect(separatorOf("")).toBe("\n");
  });

  it("displayText: textarea'nın göstereceği metin (CRLF ve tek \\r → LF)", () => {
    expect(displayText("a\r\nb\rc\nd")).toBe("a\nb\nc\nd");
    expect(displayText("")).toBe("");
  });

  it("applyDisplayEdit: değişmeyen görüntü → ham metin aynen (CRLF/karışık korunur)", () => {
    const raw = "BEGIN\r\n  x;\nEND;\r";
    expect(applyDisplayEdit(raw, displayText(raw))).toBe(raw);
  });

  it("applyDisplayEdit: CRLF belgede düzenleme dokunulmayan satır sonlarını korur, yeni satır sonu CRLF", () => {
    const raw = "BEGIN\r\n  RETURN NEW;\r\nEND;";
    expect(applyDisplayEdit(raw, "BEGIN\n  RETURN OLD;\nEND;")).toBe("BEGIN\r\n  RETURN OLD;\r\nEND;");
    expect(applyDisplayEdit(raw, "BEGIN\n  PERFORM 1;\n  RETURN NEW;\nEND;")).toBe("BEGIN\r\n  PERFORM 1;\r\n  RETURN NEW;\r\nEND;");
    // Satır sonu silme: CRLF çifti birlikte gider
    expect(applyDisplayEdit("a\r\nb", "ab")).toBe("ab");
    expect(applyDisplayEdit("a\r\n\r\nb", "a\nb")).toBe("a\r\nb");
    expect(applyDisplayEdit("a\r\nb\r\n", "a\nb")).toBe("a\r\nb");
  });

  it("applyDisplayEdit: LF belgede LF; karışık belgede yalnızca düzenlenen bölge değişir", () => {
    expect(applyDisplayEdit("a\nb", "a\nX\nb")).toBe("a\nX\nb");
    // Karışık: ilk CRLF var → ayırıcı CRLF; dokunulmayan LF ve tek \r yerinde kalır
    expect(applyDisplayEdit("a\r\nb\nc\rd", "a\nb\nc\nD")).toBe("a\r\nb\nc\rD");
    expect(applyDisplayEdit("", "x\ny")).toBe("x\ny");
    expect(applyDisplayEdit("x\r\ny", "")).toBe("");
  });
});

describe("sql-body-model: dil kipi", () => {
  it("sql / plpgsql (büyük-küçük harf duyarsız) → sql; diğerleri ve boş → plain", () => {
    expect(bodyLanguageMode("sql")).toBe("sql");
    expect(bodyLanguageMode("SQL")).toBe("sql");
    expect(bodyLanguageMode("plpgsql")).toBe("sql");
    expect(bodyLanguageMode("PLpgSQL")).toBe("sql");
    expect(bodyLanguageMode("plpython3u")).toBe("plain");
    expect(bodyLanguageMode("plperl")).toBe("plain");
    expect(bodyLanguageMode("c")).toBe("plain");
    expect(bodyLanguageMode("")).toBe("plain");
  });
});

describe("sql-body-model: girinti birimi", () => {
  it("INDENT iki boşluktur", () => {
    expect(INDENT).toBe("  ");
  });
});

describe("sql-body-model: displayDiff (dış değer → belge)", () => {
  it("aynı metin → null (dispatch yok)", () => {
    expect(displayDiff("a\nb", "a\nb")).toBeNull();
    expect(displayDiff("", "")).toBeNull();
  });

  it("ortak önek/sonek dışındaki tek değişiklik", () => {
    expect(displayDiff("BEGIN\n  x;\nEND;", "BEGIN\n  y;\nEND;")).toEqual({ from: 8, to: 9, insert: "y" });
    expect(displayDiff("ab", "aXb")).toEqual({ from: 1, to: 1, insert: "X" });
    expect(displayDiff("aXb", "ab")).toEqual({ from: 1, to: 2, insert: "" });
    expect(displayDiff("", "x")).toEqual({ from: 0, to: 0, insert: "x" });
    expect(displayDiff("abc", "")).toEqual({ from: 0, to: 3, insert: "" });
  });

  it("uygulanınca hedef metni verir", () => {
    const cases: [string, string][] = [
      ["aaa", "aa"],
      ["a\nb\nc", "a\nc"],
      ["xyz", "xyzxyz"],
      ["abc", "def"],
    ];
    for (const [a, b] of cases) {
      const d = displayDiff(a, b)!;
      expect(edit(a, d.from, d.to, d.insert)).toBe(b);
    }
  });
});

describe("sql-body-model: CodeMirror belge akışı (görüntü düzenlemesi → ham)", () => {
  // Akış: belge = displayText(ham); kullanıcı düzenlemesi belgeyi değiştirir; onChange(applyDisplayEdit(ham, belge)).
  const flow = (raw: string, from: number, to: number, insert: string) =>
    applyDisplayEdit(raw, edit(displayText(raw), from, to, insert));

  it("CRLF gövdede satır ekleme: yalnız yeni satır eklenir, diğer CRLF'ler korunur", () => {
    const raw = "BEGIN\r\n  RETURN NEW;\r\nEND;";
    // "BEGIN" sonrasında Enter + girinti + ifade (CodeMirror tek değişiklik olarak `\n  PERFORM 1;` ekler)
    expect(flow(raw, 5, 5, "\n  PERFORM 1;")).toBe("BEGIN\r\n  PERFORM 1;\r\n  RETURN NEW;\r\nEND;");
  });

  it("CRLF gövdede satır silme: CRLF çifti birlikte gider", () => {
    const raw = "BEGIN\r\n  x;\r\nEND;";
    expect(displayText(raw).slice(5, 10)).toBe("\n  x;");
    expect(flow(raw, 5, 10, "")).toBe("BEGIN\r\nEND;");
  });

  it("CRLF gövdede satır birleştirme (satır başında Backspace)", () => {
    expect(flow("a\r\nb", 1, 2, "")).toBe("ab");
  });

  it("karışık satır sonları: yalnız düzenlenen bölge değişir", () => {
    const raw = "a\r\nb\nc\rd";
    expect(flow(raw, 6, 7, "D")).toBe("a\r\nb\nc\rD");
    // LF satırının sonuna yeni satır: dokunulmayan LF ve tek \r yerinde; eklenen satır sonu ham ayırıcı (CRLF)
    expect(flow(raw, 3, 3, "\nX")).toBe("a\r\nb\nX\r\nc\rd");
  });

  it("çok satırlı Tab girintisi (indentMore: her satır başına INDENT) CRLF'yi korur", () => {
    expect(applyDisplayEdit("a\r\nb", `${INDENT}a\n${INDENT}b`)).toBe("  a\r\n  b");
  });

  it("aynı metin → aynı ham (dokunmadan = alter sabit noktası), belgeye dispatch yok", () => {
    for (const raw of ["BEGIN\r\n  x;\nEND;\r", "a\r\n", "\r", "", "x"]) {
      expect(applyDisplayEdit(raw, displayText(raw))).toBe(raw);
      expect(displayDiff(displayText(raw), displayText(raw))).toBeNull();
    }
  });
});
