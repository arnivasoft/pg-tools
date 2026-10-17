import { describe, expect, it } from "vitest";
import type { CompositeAttr } from "../../ipc/bindings/CompositeAttr";
import type { EnumValue } from "../../ipc/bindings/EnumValue";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TypeBody } from "../../ipc/bindings/TypeBody";
import type { TypeDef } from "../../ipc/bindings/TypeDef";
import {
  asType, attrAdd, attrMove, attrMoveAllowed, attrRemove, attrRestore, attrUpdate, compositeLoss, enumAdd, enumAddIndex,
  enumHasNew, enumMove, enumMoveAllowed, enumRemove, enumRemoveAllowed, enumRename, isReadOnlyVariant, newAttr, switchVariant,
  typePatch, variantOf, type TypeDraft,
} from "./type-model";

const nv = (label: string): EnumValue => ({ label, original: null });
const lv = (label: string): EnumValue => ({ label, original: label });
const na = (name: string, typeName = "text"): CompositeAttr => ({ name, typeName, collation: null, original: null });
const la = (name: string, typeName = "text"): CompositeAttr => ({ name, typeName, collation: null, original: name });

const def = (body: TypeBody, p: Partial<TypeDef> = {}): TypeDraft => ({
  kind: "type", schema: "public", name: "t", owner: null, comment: null, body, editable: true, unsupported: [], ...p,
});
const en = (values: EnumValue[]) => def({ variant: "enum", values });
const comp = (attributes: CompositeAttr[]) => def({ variant: "composite", attributes });
const range = (): TypeDraft =>
  def({ variant: "range", subtype: "int4", subtypeOpclass: null, collation: null, canonical: null, subtypeDiff: null, multirangeTypeName: null });

const labels = (d: TypeDraft) => (d.body.variant === "enum" ? d.body.values.map((v) => `${v.label}${v.original === null ? "*" : ""}`) : []);
const names = (d: TypeDraft) => (d.body.variant === "composite" ? d.body.attributes.map((a) => `${a.name}${a.original === null ? "*" : ""}`) : []);

describe("asType / typePatch / variantOf", () => {
  it("yalnız type daraltılır; patch kind/editable/unsupported korur", () => {
    const d: ObjectDef = en([lv("a")]);
    expect(asType(d)?.name).toBe("t");
    expect(asType(null)).toBeNull();
    expect(asType({ kind: "schema", name: "s", owner: null, comment: null })).toBeNull();
    const n = asType(typePatch(d, { name: "u" }));
    expect(n?.name).toBe("u");
    expect(n?.kind).toBe("type");
    expect(n?.editable).toBe(true);
    expect(n?.unsupported).toEqual([]);
    const s: ObjectDef = { kind: "schema", name: "s", owner: null, comment: null };
    expect(typePatch(s, { name: "x" })).toBe(s);
  });
  it("variantOf", () => {
    expect(variantOf(en([]))).toBe("enum");
    expect(variantOf(comp([]))).toBe("composite");
    expect(variantOf(range())).toBe("range");
  });
});

describe("switchVariant", () => {
  it("create: boş gövdeye geçer; yedekten geri yükler", () => {
    const e = en([nv("a")]);
    const stash = {};
    const c = switchVariant(e, "composite", null, stash);
    expect(c.body).toEqual({ variant: "composite", attributes: [] });
    const r = switchVariant(c, "range", null, stash);
    expect(r.body.variant).toBe("range");
    if (r.body.variant === "range") expect(r.body.subtype).toBe("");
    const back = switchVariant(r, "enum", null, stash);
    expect(labels(back)).toEqual(["a*"]);
    expect(back.editable).toBe(true);
    expect(back.unsupported).toEqual([]);
  });
  it("aynı varyant ya da alter (original var) → değişmez", () => {
    const e = en([lv("a")]);
    expect(switchVariant(e, "enum", null)).toBe(e);
    expect(switchVariant(e, "composite", e)).toBe(e);
  });
});

describe("enum", () => {
  it("enumAdd: yeni değer original=null, konumda (sıkıştırılır)", () => {
    const d = en([lv("a"), lv("b")]);
    expect(labels(enumAdd(d, 1, "x"))).toEqual(["a", "x*", "b"]);
    expect(labels(enumAdd(d, 0, "x"))).toEqual(["x*", "a", "b"]);
    expect(labels(enumAdd(d, 99, "x"))).toEqual(["a", "b", "x*"]);
    expect(labels(enumAdd(d, -3, "x"))).toEqual(["x*", "a", "b"]);
  });
  it("enumAddIndex: imleçteki satırın altı; imleç yoksa sona", () => {
    expect(enumAddIndex(3, 0)).toBe(1);
    expect(enumAddIndex(3, 2)).toBe(3);
    expect(enumAddIndex(3, null)).toBe(3);
    expect(enumAddIndex(3, 7)).toBe(3);
    expect(enumAddIndex(0, null)).toBe(0);
  });
  it("enumRename: original korunur", () => {
    const d = enumRename(en([lv("a"), nv("n")]), 0, "z");
    expect(d.body).toEqual({ variant: "enum", values: [{ label: "z", original: "a" }, nv("n")] });
    const n = enumRename(d, 1, "m");
    expect(n.body.variant === "enum" && n.body.values[1]).toEqual(nv("m"));
    expect(enumRename(d, 5, "q")).toBe(d);
  });
  it("enumRemove: yalnız yeni değer; yüklü değer reddedilir", () => {
    const d = en([lv("a"), nv("n"), lv("b")]);
    expect(enumRemoveAllowed(d, 0)).toBe(false);
    expect(enumRemoveAllowed(d, 1)).toBe(true);
    expect(enumRemove(d, 0)).toBe(d);
    expect(labels(enumRemove(d, 1))).toEqual(["a", "b"]);
    expect(enumRemove(d, 9)).toBe(d);
  });
  it("enumMove: yeni değer yüklülerin arasına ve yeniler arasında taşınır", () => {
    const d = en([lv("a"), nv("x"), nv("y"), lv("b")]);
    expect(labels(enumMove(d, 1, -1))).toEqual(["x*", "a", "y*", "b"]);
    expect(labels(enumMove(d, 1, 1))).toEqual(["a", "y*", "x*", "b"]);
    expect(labels(enumMove(d, 2, 1))).toEqual(["a", "x*", "b", "y*"]);
    // yüklü değer yeni değerle yer değiştirebilir (yüklülerin göreli sırası değişmez)
    expect(labels(enumMove(d, 3, -1))).toEqual(["a", "x*", "b", "y*"]);
  });
  it("enumMove: iki yüklü değerin yer değiştirmesi ve sınır dışı reddedilir", () => {
    const d = en([lv("a"), lv("b"), nv("x")]);
    expect(enumMoveAllowed(d, 0, 1)).toBe(false);
    expect(enumMove(d, 0, 1)).toBe(d);
    expect(enumMove(d, 1, -1)).toBe(d);
    expect(enumMoveAllowed(d, 0, -1)).toBe(false);
    expect(enumMoveAllowed(d, 2, 1)).toBe(false);
    expect(enumMoveAllowed(d, 2, -1)).toBe(true);
  });
  it("enumMove: hiçbir hamle yüklülerin göreli sırasını değiştirmez", () => {
    let d = en([lv("a"), nv("x"), lv("b"), nv("y"), lv("c")]);
    for (let k = 0; k < 40; k++) {
      const i = (k * 7) % 5;
      const dir = k % 2 === 0 ? 1 : -1;
      d = enumMove(d, i, dir);
      const loaded = d.body.variant === "enum" ? d.body.values.filter((v) => v.original !== null).map((v) => v.original) : [];
      expect(loaded).toEqual(["a", "b", "c"]);
    }
  });
  it("enumHasNew (PG < 12 ADD VALUE notu)", () => {
    expect(enumHasNew(en([lv("a")]))).toBe(false);
    expect(enumHasNew(en([lv("a"), nv("b")]))).toBe(true);
    expect(enumHasNew(comp([]))).toBe(false);
  });
  it("başka varyantta enum işlemleri no-op", () => {
    const c = comp([na("a")]);
    expect(enumAdd(c, 0, "x")).toBe(c);
    expect(enumRemove(c, 0)).toBe(c);
    expect(enumMove(c, 0, 1)).toBe(c);
  });
});

describe("composite", () => {
  it("attrAdd: yalnız sona, original=null", () => {
    const d = attrAdd(comp([la("a"), la("b")]), na("c"));
    expect(names(d)).toEqual(["a", "b", "c*"]);
    expect(names(attrAdd(d, { ...newAttr(), name: "z" }))).toEqual(["a", "b", "c*", "z*"]);
    expect(newAttr()).toEqual({ name: "", typeName: "", collation: null, original: null });
  });
  it("attrMoveAllowed: yüklü öznitelik false; yeniler kendi aralarında", () => {
    const d = comp([la("a"), la("b"), na("x"), na("y")]);
    expect(attrMoveAllowed(d, 0, 1)).toBe(false);
    expect(attrMoveAllowed(d, 1, 1)).toBe(false); // yüklü ↔ yeni (yeni yüklünün önüne geçemez)
    expect(attrMoveAllowed(d, 2, -1)).toBe(false);
    expect(attrMoveAllowed(d, 2, 1)).toBe(true);
    expect(attrMoveAllowed(d, 3, 1)).toBe(false);
    expect(names(attrMove(d, 2, 1))).toEqual(["a", "b", "y*", "x*"]);
    expect(attrMove(d, 0, 1)).toBe(d);
  });
  it("create: tüm öznitelikler yeni, serbestçe taşınır", () => {
    const d = comp([na("a"), na("b")]);
    expect(attrMoveAllowed(d, 0, 1)).toBe(true);
    expect(names(attrMove(d, 0, 1))).toEqual(["b*", "a*"]);
  });
  it("attrRemove: yüklüde DataLoss işareti; yenide yok", () => {
    const d = comp([la("a"), na("x")]);
    const r1 = attrRemove(d, 0);
    expect(names(r1.def)).toEqual(["x*"]);
    expect(r1.dataLoss).toBe(true);
    const r2 = attrRemove(d, 1);
    expect(names(r2.def)).toEqual(["a"]);
    expect(r2.dataLoss).toBe(false);
    expect(attrRemove(d, 7)).toEqual({ def: d, dataLoss: false });
  });
  it("attrUpdate: original korunur", () => {
    const d = attrUpdate(comp([la("a")]), 0, { name: "b", typeName: "int4" });
    expect(d.body.variant === "composite" && d.body.attributes[0]).toEqual({ name: "b", typeName: "int4", collation: null, original: "a" });
  });
  it("compositeLoss: silinen ve tipi/collation'ı değişen yüklü öznitelikler", () => {
    const old = comp([la("a"), la("b", "int4"), la("c")]);
    let d = attrRemove(old, 0).def;
    d = attrUpdate(d, 0, { typeName: "int8" });
    d = attrUpdate(d, 1, { collation: "C" });
    d = attrAdd(d, na("n"));
    expect(compositeLoss(old, d)).toEqual({ dropped: ["a"], retyped: ["b", "c"] });
    expect(compositeLoss(null, d)).toEqual({ dropped: [], retyped: [] });
    expect(compositeLoss(old, old)).toEqual({ dropped: [], retyped: [] });
    // yeniden adlandırma kayıp değildir
    expect(compositeLoss(old, attrUpdate(old, 0, { name: "z" }))).toEqual({ dropped: [], retyped: [] });
  });
  it("attrRestore: silinen yüklü öznitelik yüklü sırasındaki yerine döner (yeniler sonda kalır)", () => {
    const old = comp([la("a"), la("b"), la("c")]);
    let d = attrRemove(old, 1).def;
    d = attrAdd(d, na("n"));
    expect(names(attrRestore(d, old, "b"))).toEqual(["a", "b", "c", "n*"]);
    const d2 = attrRemove(attrRemove(old, 0).def, 0).def; // yalnız c
    expect(names(attrRestore(d2, old, "a"))).toEqual(["a", "c"]);
    expect(attrRestore(d2, old, "zz")).toBe(d2);
    expect(attrRestore(old, old, "a")).toBe(old); // zaten var
  });
});

describe("isReadOnlyVariant", () => {
  it("yükleyicinin boş composite + editable=false (base/multirange…) gövdesi → true", () => {
    expect(isReadOnlyVariant(def({ variant: "composite", attributes: [] }, { editable: false, unsupported: ["base type"] }))).toBe(true);
  });
  it("düzenlenebilir ya da dolu composite / enum / range → false", () => {
    expect(isReadOnlyVariant(comp([]))).toBe(false);
    expect(isReadOnlyVariant(def({ variant: "composite", attributes: [la("a")] }, { editable: false }))).toBe(false);
    expect(isReadOnlyVariant({ ...en([]), editable: false })).toBe(false);
    expect(isReadOnlyVariant({ ...range(), editable: false })).toBe(false);
  });
});
