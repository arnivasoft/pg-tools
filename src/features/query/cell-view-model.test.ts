import { describe, expect, it } from "vitest";
import {
  CELL_FORMATS, LIMITS, MiB, availability, autoFormat, byteaLength, cellFacts, columnKey, createFormatMemory, decodePrefix,
  fileActionAllowed, formatMb, isByteaHex, isPdf, looksLikeJson, looksLikeMarkup, nextTab, pickFormat, previewAllowed, sizeVerdict,
  sniffImage, sniffSvg, type CellFacts,
} from "./cell-view-model";
import { tcell } from "./strings-cell";

const hex = (bytes: number[]) => "\\x" + bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
const f = (value: string | null, typeName: string): CellFacts => cellFacts(value, { typeName });

describe("sniffImage", () => {
  it("imzaları tanır", () => {
    expect(sniffImage(new Uint8Array(PNG))).toBe("png");
    expect(sniffImage(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe("jpeg");
    expect(sniffImage(new TextEncoder().encode("GIF87a.."))).toBe("gif");
    expect(sniffImage(new TextEncoder().encode("GIF89a.."))).toBe("gif");
    expect(sniffImage(new TextEncoder().encode("RIFF\0\0\0\0WEBPVP8 "))).toBe("webp");
    expect(sniffImage(new TextEncoder().encode("BM...."))).toBe("bmp");
    expect(sniffImage(new Uint8Array([0, 0, 1, 0, 1, 0]))).toBe("ico");
    expect(sniffImage(new TextEncoder().encode("<svg xmlns='x'></svg>"))).toBe("svg");
  });
  it("tanınmayanı ve kısa diziyi reddeder", () => {
    expect(sniffImage(new TextEncoder().encode("RIFF\0\0\0\0WAVEfmt "))).toBeNull();
    expect(sniffImage(new Uint8Array([0x89, 0x50]))).toBeNull();
    expect(sniffImage(new Uint8Array([]))).toBeNull();
  });
});

describe("sniffSvg", () => {
  it("svg başlıklarını atlar", () => {
    expect(sniffSvg('<svg width="1"></svg>')).toBe(true);
    expect(sniffSvg('﻿  <?xml version="1.0"?>\n<!-- c --><!DOCTYPE svg><SVG')).toBe(true);
  });
  it("svg olmayanı reddeder", () => {
    expect(sniffSvg("<html>")).toBe(false);
    expect(sniffSvg("svg")).toBe(false);
    expect(sniffSvg("<svgfoo>")).toBe(false);
    expect(sniffSvg("<!-- açık")).toBe(false);
  });
});

describe("decodePrefix / bytea yardımcıları", () => {
  it("önek çözer", () => {
    expect(Array.from(decodePrefix("\\x89504e47", 2)!)).toEqual([0x89, 0x50]);
    expect(Array.from(decodePrefix("\\x89504E47", 9)!)).toEqual([0x89, 0x50, 0x4e, 0x47]);
    expect(decodePrefix("\\x", 4)).toEqual(new Uint8Array([]));
  });
  it("geçersizi reddeder", () => {
    expect(decodePrefix("\\x8", 2)).toBeNull();
    expect(decodePrefix("\\xgg", 2)).toBeNull();
  });
  it("isByteaHex / byteaLength / isPdf", () => {
    expect(isByteaHex("\\x00", "bytea")).toBe(true);
    expect(isByteaHex("\\x00", "text")).toBe(false);
    expect(isByteaHex("abc", "bytea")).toBe(false);
    expect(byteaLength("\\x0102")).toBe(2);
    expect(isPdf(new TextEncoder().encode("%PDF-1.7"))).toBe(true);
    expect(isPdf(new TextEncoder().encode("%PDF"))).toBe(false);
  });
});

describe("looksLike*", () => {
  it("JSON", () => {
    expect(looksLikeJson(' {"a":1}')).toBe(true);
    expect(looksLikeJson("42")).toBe(false);
    expect(looksLikeJson("{bozuk")).toBe(false);
    expect(looksLikeJson("[" + "1,".repeat(LIMITS.codeChars))).toBe(true); // > 4 MiB: yalnız ilk karakter
  });
  it("işaretleme", () => {
    expect(looksLikeMarkup("  <a/>")).toBe(true);
    expect(looksLikeMarkup("<a")).toBe(false);
    expect(looksLikeMarkup("x <a>")).toBe(false);
  });
});

describe("cellFacts", () => {
  it("bytea PNG", () => {
    const v = hex(PNG);
    expect(f(v, "bytea")).toMatchObject({ bytea: true, image: "png", pdf: false, byteLength: PNG.length, badHex: false });
  });
  it("bytea PDF", () => {
    expect(f("\\x255044462d312e37", "bytea").pdf).toBe(true);
  });
  it("bytea bozuk onaltılık", () => {
    expect(f("\\xzz", "bytea")).toMatchObject({ badHex: true, image: null, pdf: false });
  });
  it("escape çıktılı bytea metin sayılır", () => {
    expect(f("abc", "bytea")).toMatchObject({ bytea: false, badHex: false });
  });
  it("UTF-8 uzunluğu", () => {
    expect(f("ş", "text")).toMatchObject({ byteLength: 2, charLength: 1 });
    expect(f("😀", "text")).toMatchObject({ byteLength: 4, charLength: 2 });
    expect(f("a€", "text").byteLength).toBe(4);
  });
  it("JSON", () => {
    expect(f("[1,2]", "jsonb")).toMatchObject({ jsonType: true, json: true });
    expect(f("42", "jsonb").json).toBe(true);
    expect(f(' {"a":1}', "text").json).toBe(true);
    expect(f("42", "text").json).toBe(false);
    expect(f("{bozuk", "text").json).toBe(false);
  });
  it("işaretleme ve svg", () => {
    expect(f("<p>x</p>", "text").markup).toBe(true);
    expect(f('<svg xmlns="x"></svg>', "text")).toMatchObject({ markup: true, image: "svg" });
  });
  it("xml türü", () => {
    expect(f("<a>1</a>", "xml")).toMatchObject({ xmlType: true, markup: true });
    expect(f("metin", "xml")).toMatchObject({ xmlType: true, markup: false });
  });
  it("NULL", () => {
    expect(f(null, "text").isNull).toBe(true);
  });
});

describe("availability", () => {
  it("NULL: hepsi devre dışı", () => {
    const a = availability(f(null, "text"));
    for (const k of CELL_FORMATS) expect(a[k]).toEqual({ enabled: false, reason: tcell.reason.isNull });
  });
  it("text sütununda PDF/görsel", () => {
    const a = availability(f("düz", "text"));
    expect(a.text.enabled && a.hex.enabled).toBe(true);
    expect(a.pdf).toEqual({ enabled: false, reason: tcell.reason.onlyBytea });
    expect(a.image).toEqual({ enabled: false, reason: tcell.reason.notImage });
    expect(a.json).toEqual({ enabled: false, reason: tcell.reason.notJson });
    expect(a.html).toEqual({ enabled: false, reason: tcell.reason.notMarkup });
  });
  it("bytea", () => {
    const a = availability(f(hex(PNG), "bytea"));
    expect(a.image.enabled).toBe(true);
    expect(a.pdf).toEqual({ enabled: false, reason: tcell.reason.notPdf });
    expect(a.html).toEqual({ enabled: false, reason: tcell.reason.onlyText });
    expect(a.xml).toEqual({ enabled: false, reason: tcell.reason.onlyText });
    expect(availability(f("\\x255044462d", "bytea")).pdf.enabled).toBe(true);
  });
  it("bytea bozuk onaltılık", () => {
    const a = availability(f("\\xzz", "bytea"));
    expect(a.image.reason).toBe(tcell.reason.badHex);
    expect(a.pdf.reason).toBe(tcell.reason.badHex);
    expect(a.text.enabled && a.hex.enabled).toBe(true);
  });
  it("XML ve HTML", () => {
    expect(availability(f("metin", "xml")).xml.enabled).toBe(true);
    expect(availability(f("metin", "xml")).html).toEqual({ enabled: false, reason: tcell.reason.notMarkup });
    const t = availability(f("<a/>", "text"));
    expect(t.xml.enabled && t.html.enabled).toBe(true);
    expect(availability(f("<a/>", "xml")).html.enabled).toBe(true);
    expect(availability(f("42", "text")).xml).toEqual({ enabled: false, reason: tcell.reason.notXml });
  });
  it("json", () => {
    expect(availability(f('{"a":1}', "text")).json.enabled).toBe(true);
    expect(availability(f("x", "jsonb")).json.enabled).toBe(true);
  });
});

describe("autoFormat / pickFormat", () => {
  it("kurallar", () => {
    expect(autoFormat(f("[1]", "jsonb"))).toBe("json");
    expect(autoFormat(f("<a/>", "xml"))).toBe("xml");
    expect(autoFormat(f("<a/>", "text"))).toBe("text");
    expect(autoFormat(f(hex(PNG), "bytea"))).toBe("image");
    expect(autoFormat(f("\\x255044462d", "bytea"))).toBe("pdf");
    expect(autoFormat(f("\\x0102", "bytea"))).toBe("text");
    expect(autoFormat(f('{"a":1}', "text"))).toBe("text");
    expect(autoFormat(f("<p>", "text"))).toBe("text");
    expect(autoFormat(f(null, "jsonb"))).toBe("text");
  });
  it("hatırlanan", () => {
    expect(pickFormat(f("\\x0102", "bytea"), "hex")).toBe("hex");
    expect(pickFormat(f("\\x0102", "bytea"), "image")).toBe("text");
    expect(pickFormat(f("[1]", "jsonb"), undefined)).toBe("json");
    expect(pickFormat(f("[1]", "jsonb"), "text")).toBe("text");
  });
});

describe("hafıza", () => {
  it("set/get ve bağımsızlık", () => {
    const a = createFormatMemory();
    const b = createFormatMemory();
    expect(a.get("17:foto")).toBeUndefined();
    a.set("17:foto", "hex");
    expect(a.get("17:foto")).toBe("hex");
    expect(b.get("17:foto")).toBeUndefined();
  });
  it("columnKey", () => {
    expect(columnKey({ name: "foto", typeOid: 17 })).toBe("17:foto");
  });
});

describe("sizeVerdict", () => {
  const mk = (over: Partial<CellFacts>): CellFacts => ({ ...f("x", "text"), ...over });
  it("görsel", () => {
    expect(sizeVerdict("image", mk({ byteLength: 20 * MiB }))).toEqual({ ok: true });
    expect(sizeVerdict("image", mk({ byteLength: 20 * MiB + 1 }))).toEqual({ ok: false, size: 20 * MiB + 1, limit: 20 * MiB });
  });
  it("json / xml / html", () => {
    for (const k of ["json", "xml", "html"] as const) {
      expect(sizeVerdict(k, mk({ charLength: 4 * MiB }))).toEqual({ ok: true });
      expect(sizeVerdict(k, mk({ charLength: 4 * MiB + 1 })).ok).toBe(false);
    }
  });
  it("hex ve pdf", () => {
    expect(sizeVerdict("hex", mk({ byteLength: 999 * MiB }))).toEqual({ ok: true });
    expect(sizeVerdict("pdf", mk({ byteLength: 64 * MiB }))).toEqual({ ok: true });
    expect(sizeVerdict("pdf", mk({ byteLength: 64 * MiB + 1 })).ok).toBe(false);
  });
});

describe("previewAllowed / fileActionAllowed", () => {
  const mk = (over: Partial<CellFacts>): CellFacts => ({ ...f("x", "text"), ...over });
  it("önizleme", () => {
    expect(previewAllowed(mk({ charLength: 2 * MiB }))).toEqual({ enabled: true, reason: null });
    expect(previewAllowed(mk({ charLength: 2 * MiB + 1 }))).toEqual({ enabled: false, reason: tcell.html.previewTooLarge("2") });
  });
  it("dosya eylemi", () => {
    expect(fileActionAllowed(f("düz", "text"))).toEqual({ enabled: false, reason: tcell.reason.onlyBytea });
    expect(fileActionAllowed(f("\\x0102", "bytea")).enabled).toBe(true);
    expect(fileActionAllowed(mk({ bytea: true, byteLength: 64 * MiB })).enabled).toBe(true);
    const big = fileActionAllowed(mk({ bytea: true, byteLength: 64 * MiB + 1 }));
    expect(big).toEqual({ enabled: false, reason: tcell.tooLarge("64", "64") });
    expect(fileActionAllowed(f("\\xzz", "bytea")).reason).toBe(tcell.reason.badHex);
  });
});

describe("nextTab", () => {
  it("dairesel gezinme", () => {
    expect(nextTab("text", "ArrowLeft")).toBe("pdf");
    expect(nextTab("pdf", "ArrowRight")).toBe("text");
    expect(nextTab("json", "ArrowRight")).toBe("xml");
    expect(nextTab("xml", "ArrowRight")).toBe("html");
    expect(nextTab("xml", "ArrowLeft")).toBe("json");
    expect(nextTab("html", "Home")).toBe("text");
    expect(nextTab("html", "End")).toBe("pdf");
  });
});

describe("formatMb", () => {
  it("tr-TR", () => {
    expect(formatMb(1.5 * MiB)).toBe("1,5");
    expect(formatMb(20 * MiB)).toBe("20");
  });
});

describe("metinler", () => {
  it("sabitler", () => {
    expect(tcell.tabs.hex).toBe("Onaltılık");
    expect(tcell.tabs.xml).toBe("XML");
    expect(tcell.reason.notPdf).toBe("PDF değil (%PDF- ile başlamıyor).");
    expect(tcell.reason.notXml).toBe("XML işaretlemesi değil.");
    expect(tcell.pdf.info("1.7", "12.345 bayt")).toBe("PDF 1.7 · 12.345 bayt");
    expect(tcell.pdf.info(null, "5 bayt")).toBe("PDF · 5 bayt");
  });
});

describe("panel akışı", () => {
  it("sütun başına hatırlama: PNG → image; hex seçilir → imzasız hücre hex; başka sütun kendi kuralı", () => {
    const mem = createFormatMemory();
    const col = { name: "dosya", typeName: "bytea", typeOid: 17 };
    const other = { name: "belge", typeName: "bytea", typeOid: 17 };
    const png = cellFacts("\\x89504e470d0a1a0a00", col);
    expect(pickFormat(png, mem.get(columnKey(col)))).toBe("image");
    mem.set(columnKey(col), "hex");
    const plain = cellFacts("\\x0102", col);
    expect(pickFormat(plain, mem.get(columnKey(col)))).toBe("hex");
    expect(pickFormat(plain, mem.get(columnKey(other)))).toBe("text");
    mem.set(columnKey(col), "json");
    expect(pickFormat(plain, mem.get(columnKey(col)))).toBe("text");
    expect(mem.get(columnKey(col))).toBe("json");
  });
});

describe("tcell.image.info", () => {
  it("boyut yüklenince eklenir, önce atlanır", () => {
    expect(tcell.image.info("PNG", "640 × 480", "12.345 bayt")).toBe("PNG · 640 × 480 · 12.345 bayt");
    expect(tcell.image.info("SVG", null, "12.345 bayt")).toBe("SVG · 12.345 bayt");
  });
});
