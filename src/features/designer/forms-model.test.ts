import { describe, expect, it } from "vitest";
import type { ExtensionDef } from "../../ipc/bindings/ExtensionDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { SchemaDef } from "../../ipc/bindings/SchemaDef";
import { groupIssues, issuesFor } from "./designer-model";
import {
  EXTENSION_PATHS, SCHEMA_PATHS, asExtension, asSchema, extensionEditPatch, extensionOptionLabel, extensionVersionNote,
  findExtension, normalizeEmpty, schemaEditPatch,
} from "./forms-model";

const schema = (p: Partial<SchemaDef> = {}): ObjectDef => ({ kind: "schema", name: "app", owner: null, comment: null, ...p });
const ext = (p: Partial<ExtensionDef> = {}): ObjectDef => ({ kind: "extension", name: "pg_trgm", schema: null, version: "1.6", cascade: false, ...p });
const extDef = (p: Partial<ExtensionDef> = {}) => ext(p) as ExtensionDef & { kind: "extension" };

describe("normalizeEmpty", () => {
  it("boş ve yalnız boşluk → null", () => {
    expect(normalizeEmpty("")).toBeNull();
    expect(normalizeEmpty("   ")).toBeNull();
    expect(normalizeEmpty("\t\n")).toBeNull();
  });
  it("dolu metin olduğu gibi (kırpılmaz)", () => {
    expect(normalizeEmpty("public")).toBe("public");
    expect(normalizeEmpty(" a ")).toBe(" a ");
  });
});

describe("tür daraltıcılar", () => {
  it("yalnızca kendi türünü verir", () => {
    expect(asSchema(schema())?.name).toBe("app");
    expect(asSchema(ext())).toBeNull();
    expect(asSchema(null)).toBeNull();
    expect(asExtension(ext())?.name).toBe("pg_trgm");
    expect(asExtension(schema())).toBeNull();
  });
});

describe("schemaEditPatch / extensionEditPatch", () => {
  it("alanı günceller, girdiyi değiştirmez, kind korunur", () => {
    const d = schema();
    const n = schemaEditPatch(d, { name: " x ", owner: "bob" });
    expect(n).toEqual({ kind: "schema", name: " x ", owner: "bob", comment: null });
    expect(d).toEqual(schema());
    expect(n).not.toBe(d);
  });
  it("tür uyuşmazsa tanım değişmez", () => {
    const e = ext();
    expect(schemaEditPatch(e, { name: "x" })).toBe(e);
    const s = schema();
    expect(extensionEditPatch(s, { version: "2" })).toBe(s);
  });
  it("eklenti: schema/version/cascade güncellenir", () => {
    expect(extensionEditPatch(ext(), { schema: "ext", version: null, cascade: true })).toEqual({
      kind: "extension", name: "pg_trgm", schema: "ext", version: null, cascade: true,
    });
  });
});

describe("extensionOptionLabel / findExtension", () => {
  it("detail varsa o, yoksa değer", () => {
    expect(extensionOptionLabel({ value: "hstore", detail: "default 1.8 · yüklü — · key/value" })).toBe("default 1.8 · yüklü — · key/value");
    expect(extensionOptionLabel({ value: "hstore", detail: null })).toBe("hstore");
    expect(extensionOptionLabel({ value: "hstore", detail: "  " })).toBe("hstore");
  });
  it("tam eşleşme, büyük/küçük harf duyarlı", () => {
    const items = [{ value: "hstore", detail: "d" }, { value: "pg_trgm", detail: null }];
    expect(findExtension(items, "pg_trgm")).toBe(items[1]);
    expect(findExtension(items, "PG_TRGM")).toBeNull();
    expect(findExtension(items, "")).toBeNull();
  });
});

describe("extensionVersionNote", () => {
  it("alter: farklı sürüm → not; aynı ya da boş → yok", () => {
    expect(extensionVersionNote(ext({ version: "1.6" }), extDef({ version: "1.7" }))).toBe(true);
    expect(extensionVersionNote(ext({ version: "1.6" }), extDef({ version: "1.6" }))).toBe(false);
    expect(extensionVersionNote(ext({ version: "1.6" }), extDef({ version: null }))).toBe(false);
  });
  it("create (yüklenen tanım yok) → not yok", () => {
    expect(extensionVersionNote(null, extDef({ version: "1.7" }))).toBe(false);
  });
});

describe("alan yolu sözleşmesi", () => {
  it("şema: name/owner/comment alanı, diğerleri genel", () => {
    const g = groupIssues(
      [{ path: "name", message: "ad" }, { path: "owner", message: "sahip" }, { path: "old.name", message: "eski" }, { path: "schema", message: "x" }],
      SCHEMA_PATHS,
    );
    expect(issuesFor(g, "name")).toEqual(["ad"]);
    expect(issuesFor(g, "owner")).toEqual(["sahip"]);
    expect(g.general).toEqual(["eski", "x"]);
  });
  it("eklenti: name/schema/version alanı; owner genel", () => {
    const g = groupIssues(
      [{ path: "version", message: "sürüm" }, { path: "schema", message: "şema" }, { path: "owner", message: "o" }],
      EXTENSION_PATHS,
    );
    expect(issuesFor(g, "version")).toEqual(["sürüm"]);
    expect(issuesFor(g, "schema")).toEqual(["şema"]);
    expect(g.general).toEqual(["o"]);
  });
});
