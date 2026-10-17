import { describe, expect, it } from "vitest";
import type { DomainCheck } from "../../ipc/bindings/DomainCheck";
import type { DomainDef } from "../../ipc/bindings/DomainDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import {
  asDomain, canValidate, checkAdd, checkRemove, checkRename, checkRestore, checkUpdate, domainPatch, needsName, newCheck, notValidAllowed,
  removedChecks,
  type DomainDraft,
} from "./domain-model";

const lc = (name: string, expression = "VALUE > 0", validated = true): DomainCheck => ({
  name, expression, notValid: false, validated, original: name, validate: false,
});
const nc = (name: string | null, expression = "VALUE > 0"): DomainCheck => ({
  name, expression, notValid: false, validated: false, original: null, validate: false,
});
const def = (constraints: DomainCheck[], p: Partial<DomainDef> = {}): DomainDraft => ({
  kind: "domain", schema: "public", name: "d", owner: null, comment: null, baseType: "integer", collation: null,
  default: null, notNull: false, constraints, editable: true, unsupported: [], ...p,
});

describe("asDomain / domainPatch", () => {
  it("yalnız domain daraltılır; patch kind/editable/unsupported korur", () => {
    const d: ObjectDef = def([]);
    expect(asDomain(d)?.name).toBe("d");
    expect(asDomain(null)).toBeNull();
    const s: ObjectDef = { kind: "schema", name: "s", owner: null, comment: null };
    expect(asDomain(s)).toBeNull();
    const n = asDomain(domainPatch(d, { default: "0", notNull: true }));
    expect(n?.default).toBe("0");
    expect(n?.notNull).toBe(true);
    expect(n?.kind).toBe("domain");
    expect(n?.editable).toBe(true);
    expect(n?.unsupported).toEqual([]);
    expect(domainPatch(s, { name: "x" })).toBe(s);
  });
});

describe("checkAdd / checkRemove / checkRename (değişmez)", () => {
  it("checkAdd sona boş yeni kısıt ekler; girdi değişmez", () => {
    const d = def([lc("a")]);
    const n = checkAdd(d);
    expect(n.constraints).toHaveLength(2);
    expect(n.constraints[1]).toEqual({ name: "", expression: "", notValid: false, validated: false, original: null, validate: false });
    expect(newCheck()).toEqual(n.constraints[1]);
    expect(d.constraints).toHaveLength(1);
    expect(n).not.toBe(d);
  });
  it("checkRemove satırı siler; sınır dışı → aynı nesne", () => {
    const d = def([lc("a"), nc("b")]);
    const n = checkRemove(d, 0);
    expect(n.constraints.map((c) => c.name)).toEqual(["b"]);
    expect(d.constraints).toHaveLength(2);
    expect(checkRemove(d, 5)).toBe(d);
    expect(checkRemove(d, -1)).toBe(d);
  });
  it("checkRename adı değiştirir, original korunur; boş → null; sınır dışı → aynı nesne", () => {
    const d = def([lc("a")]);
    const n = checkRename(d, 0, "b");
    expect(n.constraints[0]).toMatchObject({ name: "b", original: "a" });
    expect(d.constraints[0]!.name).toBe("a");
    expect(checkRename(d, 0, "  ").constraints[0]!.name).toBeNull();
    expect(checkRename(d, 3, "x")).toBe(d);
  });
});

describe("needsName", () => {
  it("alter'da adsız kısıt (yeni) ad ister; create'te serbest", () => {
    expect(needsName("alter", nc(null))).toBe(true);
    expect(needsName("alter", nc(""))).toBe(true);
    expect(needsName("alter", nc("x"))).toBe(false);
    expect(needsName("create", nc(null))).toBe(false);
    expect(needsName("create", nc(""))).toBe(false);
    expect(needsName("alter", lc("a"))).toBe(false);
  });
});

describe("canValidate / notValidAllowed", () => {
  const original = def([lc("a", "VALUE > 0", false), lc("b", "VALUE < 9", true)]);
  it("canValidate: yüklü + doğrulanmamış + ifade değişmemiş", () => {
    expect(canValidate(lc("a", "VALUE > 0", false), original)).toBe(true);
    // yeniden adlandırılmış olsa da (original eşlemesi) doğrulanabilir
    expect(canValidate({ ...lc("a", "VALUE > 0", false), name: "a2" }, original)).toBe(true);
    expect(canValidate(lc("a", "VALUE > 1", false), original)).toBe(false);
    expect(canValidate(lc("b", "VALUE < 9", true), original)).toBe(false);
    expect(canValidate(nc("n"), original)).toBe(false);
    expect(canValidate(lc("a", "VALUE > 0", false), null)).toBe(false);
    expect(canValidate(lc("zz", "VALUE > 0", false), original)).toBe(false);
  });
  it("notValidAllowed: yalnız alter'da yeni ya da ifadesi değişmiş kısıt", () => {
    expect(notValidAllowed(nc("n"), original)).toBe(true);
    expect(notValidAllowed(lc("a", "VALUE > 5", false), original)).toBe(true);
    expect(notValidAllowed(lc("a", "VALUE > 0", false), original)).toBe(false);
    expect(notValidAllowed(nc("n"), null)).toBe(false);
  });
  it("checkUpdate: izin kalkınca notValid/validate sıfırlanır", () => {
    const d = def([{ ...lc("a", "VALUE > 5", false), notValid: true }]);
    const back = checkUpdate(d, 0, { expression: "VALUE > 0" }, original);
    expect(back.constraints[0]).toMatchObject({ expression: "VALUE > 0", notValid: false, original: "a" });
    const v = checkUpdate(back, 0, { validate: true }, original);
    expect(v.constraints[0]!.validate).toBe(true);
    const changed = checkUpdate(v, 0, { expression: "VALUE > 7" }, original);
    expect(changed.constraints[0]!.validate).toBe(false);
    expect(checkUpdate(d, 0, { notValid: true }, null).constraints[0]!.notValid).toBe(false);
    expect(checkUpdate(d, 9, { expression: "x" }, original)).toBe(d);
    expect(d.constraints[0]!.notValid).toBe(true);
  });
});

describe("removedChecks / checkRestore (silinen yüklü kısıtı geri al)", () => {
  const orig = def([lc("a"), lc("b", "VALUE < 9", false)]);
  it("removedChecks: taslakta eşi olmayan yüklü adlar (yüklü sırada); create (original yok) → boş", () => {
    expect(removedChecks(orig, def([lc("a"), lc("b")]))).toEqual([]);
    expect(removedChecks(orig, def([lc("b")]))).toEqual(["a"]);
    expect(removedChecks(orig, def([nc("a")]))).toEqual(["a", "b"]);
    // Yeniden adlandırılan yüklü kısıt (`original` korunur) silinmiş sayılmaz.
    expect(removedChecks(orig, def([{ ...lc("a"), name: "a2" }, lc("b")]))).toEqual([]);
    expect(removedChecks(null, def([]))).toEqual([]);
  });
  it("checkRestore: yüklü kısıt olduğu gibi sona eklenir; zaten varsa ya da bilinmiyorsa aynı nesne", () => {
    const d = def([nc("x")]);
    const r = checkRestore(d, orig, "b");
    expect(r.constraints).toEqual([nc("x"), lc("b", "VALUE < 9", false)]);
    expect(removedChecks(orig, r)).toEqual(["a"]);
    expect(d.constraints).toHaveLength(1);
    expect(checkRestore(r, orig, "b")).toBe(r);
    expect(checkRestore(d, orig, "zz")).toBe(d);
    expect(checkRestore(d, null, "a")).toBe(d);
  });
});
