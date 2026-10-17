import { describe, expect, it } from "vitest";
import { INDENT, applyDisplayEdit, displayText, separatorOf, tabEdit } from "./sql-body-model";

/** `tabEdit` sonucunu metne uygular. */
function run(text: string, start: number, end: number, outdent = false) {
  const e = tabEdit(text, start, end, outdent);
  if (!e) return null;
  return { text: text.slice(0, e.from) + e.insert + text.slice(e.to), sel: [e.selStart, e.selEnd] };
}

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

describe("sql-body-model: Tab girintisi (iki boşluk)", () => {
  it("INDENT iki boşluktur", () => {
    expect(INDENT).toBe("  ");
  });

  it("seçimsiz Tab imlece iki boşluk ekler", () => {
    expect(run("BEGIN\nEND;", 6, 6)).toEqual({ text: "BEGIN\n  END;", sel: [8, 8] });
    expect(run("ab", 1, 1)).toEqual({ text: "a  b", sel: [3, 3] });
  });

  it("seçimli Tab dokunduğu her satırı girintiler; seçim satırları kapsamaya devam eder", () => {
    const t = "a\nb\nc";
    expect(run(t, 0, 3)).toEqual({ text: "  a\n  b\nc", sel: [0, 7] });
    // Seçim sonu bir satırın başındaysa o satır dahil edilmez
    expect(run(t, 0, 4)).toEqual({ text: "  a\n  b\nc", sel: [0, 8] });
    // Tek satır içinde seçim: satır girintilenir, seçim kayar
    expect(run("abc", 1, 2)).toEqual({ text: "  abc", sel: [3, 4] });
  });

  it("Shift+Tab: satır(lar)ın başından en çok iki boşluk ya da bir sekme kaldırır", () => {
    expect(run("    a", 4, 4, true)).toEqual({ text: "  a", sel: [2, 2] });
    expect(run(" a", 1, 1, true)).toEqual({ text: "a", sel: [0, 0] });
    expect(run("\ta", 1, 1, true)).toEqual({ text: "a", sel: [0, 0] });
    expect(run("  a\n  b\nc", 0, 7, true)).toEqual({ text: "a\nb\nc", sel: [0, 3] });
    // İmleç girinti içindeyse satır başına sınırlanır
    expect(run("  a", 1, 1, true)).toEqual({ text: "a", sel: [0, 0] });
  });

  it("Shift+Tab: kaldırılacak girinti yoksa null (metin değişmez)", () => {
    expect(tabEdit("a\nb", 0, 3, true)).toBeNull();
  });

  it("CRLF içeren ham metin: girinti görüntü metninde yapılır, applyDisplayEdit CRLF'yi korur", () => {
    const raw = "a\r\nb";
    const shown = displayText(raw);
    const r = run(shown, 0, shown.length)!;
    expect(applyDisplayEdit(raw, r.text)).toBe("  a\r\n  b");
  });
});
