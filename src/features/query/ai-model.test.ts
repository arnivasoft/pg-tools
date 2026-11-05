import { describe, expect, it } from "vitest";
import type { CompletionItem } from "../../ipc/bindings/CompletionItem";
import { mentionedRelations, schemaText, sliceChars, splitBlocks } from "./ai-model";

const rel = (label: string): CompletionItem => ({ label, quoted: label, kind: "table", detail: null });

describe("splitBlocks", () => {
  it("metin ve kapalı SQL bloklarını ayırır", () => {
    const b = splitBlocks("Açıklama.\n\n```sql\nSELECT 1;\n```\nSon not.");
    expect(b).toEqual([
      { kind: "text", text: "Açıklama." },
      { kind: "code", lang: "sql", text: "SELECT 1;", open: false },
      { kind: "text", text: "Son not." },
    ]);
  });

  it("akış sürerken kapanmamış bloğu açık işaretler", () => {
    expect(splitBlocks("x\n```SQL\nSELECT")).toEqual([
      { kind: "text", text: "x" },
      { kind: "code", lang: "sql", text: "SELECT", open: true },
    ]);
  });

  it("çitsiz metin tek bloktur; boş metin boş dizidir", () => {
    expect(splitBlocks("merhaba")).toEqual([{ kind: "text", text: "merhaba" }]);
    expect(splitBlocks("")).toEqual([]);
  });

  it("birden çok blok ve CRLF", () => {
    const b = splitBlocks("```sql\r\nA\r\n```\r\narada\r\n```\r\nB\r\n```");
    expect(b.map((x) => x.text)).toEqual(["A", "arada", "B"]);
  });
});

describe("sliceChars", () => {
  it("vekil çiftleri tek karakter sayar", () => {
    expect(sliceChars("a😀b select", 2, 8)).toBe("b select");
  });
});

describe("mentionedRelations", () => {
  it("tam sözcük ve harf duyarsız eşleşir", () => {
    const rels = [rel("orders"), rel("order_items"), rel("Users")];
    expect(mentionedRelations("Son ORDERS ve users kayıtları", rels).map((r) => r.label)).toEqual(["orders", "Users"]);
  });
});

describe("schemaText", () => {
  it("ilişkiler ve kolonlar", () => {
    const cols = new Map([["orders", [{ label: "id", quoted: "id", kind: "column", detail: "int4" } as CompletionItem]]]);
    expect(schemaText([rel("orders")], cols)).toBe("Relations in search_path: orders (table)\norders: id int4");
    expect(schemaText([], new Map())).toBe("");
  });
});
