import { describe, expect, it } from "vitest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PartitionDef } from "../../ipc/bindings/PartitionDef";
import {
  asPartition, attachPick, boundChanged, defaultPatch, emptyBound, hashPatch, listAdd, listRemove, listValuePatch, modePatch,
  partitionPatch, rangeKindPatch, rangeValuePatch, subKeyKind, subKeyPatch, subKeysPatch, subStrategyPatch,
} from "./partition-model";

const part = (p: Partial<PartitionDef> = {}): PartitionDef => ({
  parentSchema: "public", parentTable: "t", strategy: "range", keyCount: 2, mode: "create", schema: "public", name: "t_p1",
  bound: emptyBound("range", 2), partitionBy: null, tablespace: null, owner: null, comment: null, detachPending: false,
  editable: true, unsupported: [], ...p,
});
const range = (from: string[], to: string[]) =>
  part({ bound: { kind: "range", from: from.map((sql) => ({ kind: "value", sql })), to: to.map((sql) => ({ kind: "value", sql })) } });

describe("asPartition / partitionPatch", () => {
  it("başka türde null; tür uyuşmazlığında aynı nesne", () => {
    const s: ObjectDef = { kind: "schema", name: "a", owner: null, comment: null };
    expect(asPartition(s)).toBeNull();
    expect(asPartition(null)).toBeNull();
    expect(partitionPatch(s, { name: "x" })).toBe(s);
    const p: ObjectDef = { ...part(), kind: "partition" };
    expect(asPartition(p)).toBe(p);
    expect(partitionPatch(p, { name: "x" })).toMatchObject({ kind: "partition", name: "x" });
  });
});

describe("emptyBound", () => {
  it("range: keyCount boş value; list: []; hash: 1/0", () => {
    for (const n of [1, 3]) {
      const b = emptyBound("range", n);
      expect(b).toEqual({
        kind: "range", from: Array.from({ length: n }, () => ({ kind: "value", sql: "" })), to: Array.from({ length: n }, () => ({ kind: "value", sql: "" })),
      });
    }
    expect(emptyBound("list", 3)).toEqual({ kind: "list", values: [] });
    expect(emptyBound("hash", 1)).toEqual({ kind: "hash", modulus: 1, remainder: 0 });
  });
  it("from ve to ayrı diziler", () => {
    const b = emptyBound("range", 1);
    if (b.kind === "range") expect(b.from).not.toBe(b.to);
  });
});

describe("modePatch", () => {
  it("attach: alt bölümleme/tablespace/sahip/yorum temizlenir; girdi değişmez", () => {
    const d = part({ partitionBy: { strategy: "list", keys: [{ kind: "column", name: "a" }] }, tablespace: "ts", owner: "o", comment: "c" });
    const snap = JSON.stringify(d);
    const a = modePatch(d, "attach");
    expect(a).toMatchObject({ mode: "attach", partitionBy: null, tablespace: null, owner: null, comment: null, name: "t_p1" });
    expect(JSON.stringify(d)).toBe(snap);
  });
  it("create'e dönüş: ad boş, şema üst şema", () => {
    const d = part({ mode: "attach", schema: "other", name: "x" });
    expect(modePatch(d, "create")).toMatchObject({ mode: "create", name: "", schema: "public" });
  });
  it("aynı kip aynı nesne", () => {
    const d = part();
    expect(modePatch(d, "create")).toBe(d);
  });
});

describe("defaultPatch", () => {
  it("aç → default; kapat → iskelet", () => {
    const d = part();
    const on = defaultPatch(d, true);
    expect(on.bound).toEqual({ kind: "default" });
    expect(defaultPatch(on, false).bound).toEqual(emptyBound("range", 2));
    expect(defaultPatch(part({ strategy: "list", bound: { kind: "default" }, keyCount: 1 }), false).bound).toEqual({ kind: "list", values: [] });
  });
  it("zaten o durumda aynı nesne", () => {
    const d = part();
    expect(defaultPatch(d, false)).toBe(d);
  });
});

describe("range", () => {
  it("rangeKindPatch: value→minvalue metni atar; minvalue→value boş; value→value korur", () => {
    const d = range(["10", "20"], ["30", "40"]);
    const m = rangeKindPatch(d, "from", 0, "minvalue");
    expect(m.bound).toMatchObject({ from: [{ kind: "minvalue" }, { kind: "value", sql: "20" }] });
    const v = rangeKindPatch(m, "from", 0, "value");
    expect(v.bound).toMatchObject({ from: [{ kind: "value", sql: "" }, { kind: "value", sql: "20" }] });
    expect(rangeKindPatch(d, "to", 1, "value")).toBe(d);
    const x = rangeKindPatch(d, "to", 1, "maxvalue");
    expect(x.bound).toMatchObject({ to: [{ kind: "value", sql: "30" }, { kind: "maxvalue" }] });
  });
  it("indeks dışı ve list'te no-op", () => {
    const d = range(["1"], ["2"]);
    expect(rangeKindPatch(d, "from", 5, "minvalue")).toBe(d);
    expect(rangeKindPatch(d, "from", -1, "minvalue")).toBe(d);
    const l = part({ strategy: "list", bound: { kind: "list", values: ["a"] } });
    expect(rangeKindPatch(l, "from", 0, "minvalue")).toBe(l);
    expect(rangeValuePatch(l, "from", 0, "x")).toBe(l);
  });
  it("rangeValuePatch metni kırpmaz", () => {
    const d = range(["", ""], ["", ""]);
    expect(rangeValuePatch(d, "to", 1, "  '2024-01-01' ").bound).toMatchObject({ to: [{ sql: "" }, { kind: "value", sql: "  '2024-01-01' " }] });
    expect(rangeValuePatch(d, "to", 9, "x")).toBe(d);
  });
});

describe("list", () => {
  const l = part({ strategy: "list", bound: { kind: "list", values: ["a", "b"] } });
  it("listAdd sona boş ekler", () => {
    expect(listAdd(l).bound).toEqual({ kind: "list", values: ["a", "b", ""] });
    const r = part();
    expect(listAdd(r)).toBe(r);
  });
  it("listRemove / listValuePatch; indeks dışı no-op", () => {
    expect(listRemove(l, 0).bound).toEqual({ kind: "list", values: ["b"] });
    expect(listRemove(l, 2)).toBe(l);
    expect(listRemove(l, -1)).toBe(l);
    expect(listValuePatch(l, 1, " NULL ").bound).toEqual({ kind: "list", values: ["a", " NULL "] });
    expect(listValuePatch(l, 2, "x")).toBe(l);
  });
});

describe("hashPatch", () => {
  const h = part({ strategy: "hash", bound: { kind: "hash", modulus: 1, remainder: 0 } });
  it("geçerli sayıyı yazar", () => {
    expect(hashPatch(h, "modulus", "4").bound).toEqual({ kind: "hash", modulus: 4, remainder: 0 });
    expect(hashPatch(h, "remainder", "0").bound).toEqual({ kind: "hash", modulus: 1, remainder: 0 });
    expect(hashPatch(h, "modulus", "4294967295").bound).toMatchObject({ modulus: 4294967295 });
  });
  it("geçersiz metin aynı nesne", () => {
    for (const t of ["x", "-1", "4294967296", "", "1.5", " 3"]) expect(hashPatch(h, "modulus", t)).toBe(h);
    const r = part();
    expect(hashPatch(r, "modulus", "2")).toBe(r);
  });
});

describe("boundChanged", () => {
  it("create'te false; eşit false", () => {
    const d = range(["1"], ["2"]);
    expect(boundChanged(null, d)).toBe(false);
    expect(boundChanged(d, structuredClone(d))).toBe(false);
  });
  it("değer farkı ve tür farkı true", () => {
    const a = range(["1"], ["2"]);
    expect(boundChanged(a, range(["1"], ["3"]))).toBe(true);
    expect(boundChanged(a, rangeKindPatch(a, "from", 0, "minvalue"))).toBe(true);
    const l = part({ strategy: "list", bound: { kind: "list", values: ["a"] } });
    expect(boundChanged(l, defaultPatch(l, true))).toBe(true);
    expect(boundChanged(defaultPatch(l, true), l)).toBe(true);
    expect(boundChanged(l, listValuePatch(l, 0, "b"))).toBe(true);
    const h = part({ strategy: "hash", bound: { kind: "hash", modulus: 4, remainder: 1 } });
    expect(boundChanged(h, hashPatch(h, "remainder", "2"))).toBe(true);
    expect(boundChanged(h, structuredClone(h))).toBe(false);
  });
});

describe("attachPick", () => {
  it("şema ve ad yazılır", () => {
    expect(attachPick(part({ mode: "attach", name: "", schema: "public" }), "old", "arch")).toMatchObject({ schema: "arch", name: "old" });
  });
});

describe("alt bölümleme", () => {
  it("strateji seçimi tek boş anahtar kurar; null temizler; anahtarlar korunur", () => {
    const a = subStrategyPatch(part(), "list");
    expect(a.partitionBy).toEqual({ strategy: "list", keys: [{ kind: "column", name: "" }] });
    const b = subStrategyPatch(subKeyPatch(a, 0, { kind: "column", name: "x" }), "hash");
    expect(b.partitionBy).toEqual({ strategy: "hash", keys: [{ kind: "column", name: "x" }] });
    expect(subStrategyPatch(b, null).partitionBy).toBeNull();
    const n = part();
    expect(subStrategyPatch(n, null)).toBe(n);
  });
  it("anahtar listesi ve anahtar yazımı; indeks dışı no-op; alt bölümleme yoksa no-op", () => {
    const a = subStrategyPatch(part(), "range");
    const two = subKeysPatch(a, [{ kind: "column", name: "a" }, { kind: "expression", sql: "b + 1" }]);
    expect(two.partitionBy?.keys).toHaveLength(2);
    expect(subKeyPatch(two, 1, { kind: "column", name: "z" }).partitionBy?.keys[1]).toEqual({ kind: "column", name: "z" });
    expect(subKeyPatch(two, 5, { kind: "column", name: "z" })).toBe(two);
    const n = part();
    expect(subKeysPatch(n, [])).toBe(n);
    expect(subKeyPatch(n, 0, { kind: "column", name: "z" })).toBe(n);
  });
  it("subKeyKind metni taşır", () => {
    expect(subKeyKind({ kind: "column", name: "a" }, "expression")).toEqual({ kind: "expression", sql: "a" });
    expect(subKeyKind({ kind: "expression", sql: "lower(a)" }, "column")).toEqual({ kind: "column", name: "lower(a)" });
    const k = { kind: "column", name: "a" } as const;
    expect(subKeyKind(k, "column")).toBe(k);
  });
});
