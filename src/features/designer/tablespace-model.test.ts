import { describe, expect, it } from "vitest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TablespaceDef } from "../../ipc/bindings/TablespaceDef";
import type { TablespaceOptions } from "../../ipc/bindings/TablespaceOptions";
import { asTablespace, tablespaceOptionPatch, tablespaceOptionWarnings, tablespacePatch } from "./tablespace-model";

const opts = (p: Partial<TablespaceOptions> = {}): TablespaceOptions => ({
  seqPageCost: null, randomPageCost: null, effectiveIoConcurrency: null, maintenanceIoConcurrency: null, ...p,
});
const ts = (p: Partial<TablespaceDef> = {}): TablespaceDef => ({
  name: "ts1", owner: null, location: "/data/ts1", options: opts(), comment: null, editable: true, unsupported: [], ...p,
});

describe("asTablespace / tablespacePatch", () => {
  it("yalnız tablespace döner", () => {
    expect(asTablespace(null)).toBeNull();
    expect(asTablespace({ kind: "schema", name: "a", owner: null, comment: null })).toBeNull();
    expect(asTablespace({ kind: "tablespace", ...ts() })?.name).toBe("ts1");
  });
  it("alanı yazar, kind korunur; tür uyuşmazlığında aynı nesne", () => {
    const d: ObjectDef = { kind: "tablespace", ...ts() };
    const r = tablespacePatch(d, { name: "x" });
    expect(r).toMatchObject({ kind: "tablespace", name: "x", location: "/data/ts1" });
    const s: ObjectDef = { kind: "schema", name: "a", owner: null, comment: null };
    expect(tablespacePatch(s, { name: "x" })).toBe(s);
  });
});

describe("tablespaceOptionPatch", () => {
  it("kırpar", () => {
    expect(tablespaceOptionPatch(ts(), "seqPageCost", "  1.5 ").options.seqPageCost).toBe("1.5");
  });
  it("boş / yalnız boşluk → null", () => {
    const d = ts({ options: opts({ seqPageCost: "1" }) });
    expect(tablespaceOptionPatch(d, "seqPageCost", "").options.seqPageCost).toBeNull();
    expect(tablespaceOptionPatch(d, "seqPageCost", "   ").options.seqPageCost).toBeNull();
  });
  it("diğer anahtarlar korunur, girdi değişmez", () => {
    const d = ts({ options: opts({ randomPageCost: "4", effectiveIoConcurrency: "2" }) });
    const r = tablespaceOptionPatch(d, "seqPageCost", "1");
    expect(r.options).toEqual(opts({ seqPageCost: "1", randomPageCost: "4", effectiveIoConcurrency: "2" }));
    expect(d.options.seqPageCost).toBeNull();
    expect(r).not.toBe(d);
    expect(r.location).toBe("/data/ts1");
  });
});

describe("tablespaceOptionWarnings", () => {
  const w = (p: Partial<TablespaceOptions>) => tablespaceOptionWarnings(opts(p));
  it("boş / null uyarı üretmez", () => {
    expect(w({})).toEqual({});
    expect(w({ seqPageCost: "", effectiveIoConcurrency: " " })).toEqual({});
  });
  it("cost alanları", () => {
    expect(w({ seqPageCost: "1.5" })).toEqual({});
    expect(w({ seqPageCost: "1e3" })).toEqual({});
    expect(w({ randomPageCost: "abc" })).toEqual({ randomPageCost: "notNumber" });
    expect(w({ randomPageCost: "-1" })).toEqual({ randomPageCost: "negative" });
  });
  it("io alanları", () => {
    expect(w({ effectiveIoConcurrency: "1.5" })).toEqual({ effectiveIoConcurrency: "notInteger" });
    expect(w({ effectiveIoConcurrency: "abc" })).toEqual({ effectiveIoConcurrency: "notNumber" });
    expect(w({ maintenanceIoConcurrency: "-1" })).toEqual({ maintenanceIoConcurrency: "negative" });
    expect(w({ maintenanceIoConcurrency: "1001" })).toEqual({ maintenanceIoConcurrency: "outOfRange" });
    expect(w({ maintenanceIoConcurrency: "1000" })).toEqual({});
    expect(w({ maintenanceIoConcurrency: "0" })).toEqual({});
  });
});
