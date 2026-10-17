import { describe, expect, it } from "vitest";
import type { ArgMode } from "../../ipc/bindings/ArgMode";
import type { FunctionArg } from "../../ipc/bindings/FunctionArg";
import type { FunctionDef } from "../../ipc/bindings/FunctionDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import {
  argModes, asRoutine, hasRecreateError, languageChoices, recreateLoss, recreateNeeds, returnColumns, returnKindOf, returnTypeName, routinePatch, rowsVisible,
  procedureHiddenErrors, securityDefinerWarning, shouldFocusRecreate, stashReturn, syncRecreate, tableOutArgsWarning, withReturns,
} from "./routine-model";

const f = (p: Partial<FunctionDef> = {}): FunctionDef => ({
  schema: "public", name: "f", owner: null, comment: null, language: "plpgsql", args: [], returns: { kind: "void" },
  body: "BEGIN END;", volatility: "volatile", strict: false, securityDefiner: false, leakproof: false, parallel: "unsafe",
  cost: null, rows: null, settings: [], recreate: false, editable: true, unsupported: [], ...p,
});
const arg = (mode: ArgMode, name: string | null, typeName: string, def: string | null = null): FunctionArg => ({
  mode, name, typeName, default: def,
});
const ty = (typeName: string, setof = false): FunctionDef["returns"] => ({ kind: "type", typeName, setof });

describe("asRoutine / routinePatch", () => {
  it("function ve procedure daraltılır; diğer türler null", () => {
    const fn: ObjectDef = { kind: "function", ...f() };
    const pr: ObjectDef = { kind: "procedure", ...f({ returns: null }) };
    expect(asRoutine(fn)?.kind).toBe("function");
    expect(asRoutine(pr)?.kind).toBe("procedure");
    expect(asRoutine(null)).toBeNull();
    expect(asRoutine({ kind: "schema", name: "s", owner: null, comment: null })).toBeNull();
  });
  it("routinePatch: aynı tür → alanlar güncellenir (kind korunur); tür uyuşmazlığında aynı nesne", () => {
    const fn: ObjectDef = { kind: "function", ...f() };
    const next = routinePatch(fn, { name: "g" });
    expect(next).toMatchObject({ kind: "function", name: "g" });
    expect(fn).toMatchObject({ name: "f" });
    const sc: ObjectDef = { kind: "schema", name: "s", owner: null, comment: null };
    expect(routinePatch(sc, { name: "g" })).toBe(sc);
  });
});

describe("recreateNeeds (spec §8.2 aynası)", () => {
  const base = f({
    args: [arg("in", "a", "integer"), arg("in", "b", "text", "'x'"), arg("out", "o", "integer")],
    returns: ty("integer"),
  });
  const [a0, a1, a2] = base.args as [FunctionArg, FunctionArg, FunctionArg];
  const withArgs = (args: FunctionArg[]) => ({ ...base, args });

  it("create (old = null) → []", () => {
    expect(recreateNeeds(null, base)).toEqual([]);
  });
  it("değişiklik yoksa ya da yalnız gövde/öznitelik → []", () => {
    expect(recreateNeeds(base, base)).toEqual([]);
    expect(recreateNeeds(base, { ...base, body: "BEGIN o := 2; END", strict: true, cost: "5" })).toEqual([]);
  });
  const cases: Array<[string, FunctionDef, string[]]> = [
    ["giriş tipi metin değişimi", withArgs([arg("in", "a", "bigint"), a1, a2]), ["args[0].typeName"]],
    // İstemci yalnız metin karşılaştırır (int4 ↔ integer eşdeğerliği arka uca bırakılır; kutu gereksiz görünebilir).
    ["tip takma adı (metin farklı)", withArgs([arg("in", "a", "int4"), a1, a2]), ["args[0].typeName"]],
    ["argüman ekleme", withArgs([a0, a1, arg("in", "c", "int", "1"), a2]), ["args"]],
    ["argüman silme", withArgs([a0, a2]), ["args"]],
    ["var olan ad değişimi", withArgs([arg("in", "z", "integer"), a1, a2]), ["args[0].name"]],
    ["var olan adı kaldırma", withArgs([arg("in", null, "integer"), a1, a2]), ["args[0].name"]],
    ["varsayılan kaldırma", withArgs([a0, arg("in", "b", "text"), a2]), ["args[1].default"]],
    ["OUT tipi", withArgs([a0, a1, arg("out", "o", "text")]), ["args"]],
    ["OUT adı", withArgs([a0, a1, arg("out", "p", "integer")]), ["args"]],
    ["OUT kaldırma", withArgs([a0, a1]), ["args"]],
    ["OUT ekleme", withArgs([a0, a1, a2, arg("out", "q", "text")]), ["args"]],
    ["IN → INOUT", withArgs([arg("inout", "a", "integer"), a1, a2]), ["args"]],
    ["dönüş tipi", { ...base, returns: ty("bigint") }, ["returns"]],
    ["SETOF", { ...base, returns: ty("integer", true) }, ["returns"]],
    ["void", { ...base, returns: { kind: "void" } }, ["returns"]],
    ["imza + dönüş", { ...base, args: [arg("in", "a", "bigint"), a1, a2], returns: ty("text") }, ["args[0].typeName", "returns"]],
    // Tip değişimi raporlandıysa ad değişimi ayrıca raporlanmaz (arka uç `else if`).
    ["tip + ad", withArgs([arg("in", "z", "bigint"), a1, a2]), ["args[0].typeName"]],
  ];
  for (const [label, next, want] of cases) {
    it(label, () => expect(recreateNeeds(base, next)).toEqual(want));
  }
  it("serbest: varsayılan ekleme/değiştirme, adsıza ad verme", () => {
    expect(recreateNeeds(base, withArgs([a0, arg("in", "b", "text", "'y'"), a2]))).toEqual([]);
    expect(recreateNeeds(base, withArgs([arg("in", "a", "integer", "0"), a1, a2]))).toEqual([]);
    const unnamed = f({ args: [arg("in", null, "integer")] });
    expect(recreateNeeds(unnamed, f({ args: [arg("in", "a", "integer")] }))).toEqual([]);
  });
  it("yol indeksi taslaktaki sıradır (OUT girişlerin arasında)", () => {
    const old = f({ args: [arg("out", "o", "integer"), arg("in", "a", "integer")] });
    expect(recreateNeeds(old, f({ args: [arg("out", "o", "integer"), arg("in", "a", "bigint")] }))).toEqual(["args[1].typeName"]);
  });
  it("INOUT ad değişimi: giriş olarak raporlanır, çıkış listesi ayrıca `args` üretmez", () => {
    const old = f({ args: [arg("inout", "a", "integer")] });
    expect(recreateNeeds(old, f({ args: [arg("inout", "b", "integer")] }))).toEqual(["args[0].name"]);
  });
  it("TABLE kolonları dönüş tipidir; procedure (returns null) karşılaştırması", () => {
    const t = (n: string) => f({ returns: { kind: "table", columns: [{ name: n, typeName: "integer" }] } });
    expect(recreateNeeds(t("a"), t("b"))).toEqual(["returns"]);
    expect(recreateNeeds(t("a"), t("a"))).toEqual([]);
    expect(recreateNeeds(f({ returns: null }), f({ returns: null }))).toEqual([]);
    expect(recreateNeeds(f({ returns: { kind: "trigger" } }), f({ returns: { kind: "trigger" } }))).toEqual([]);
  });
});

describe("syncRecreate", () => {
  it("gereksinim kalmadıysa recreate false'a çekilir; varsa korunur", () => {
    const old = f({ args: [arg("in", "a", "integer")] });
    expect(syncRecreate(old, { ...old, recreate: true }).recreate).toBe(false);
    const changed = { ...old, args: [arg("in", "a", "bigint")], recreate: true };
    expect(syncRecreate(old, changed)).toBe(changed);
    const plain = { ...old, body: "x" };
    expect(syncRecreate(old, plain)).toBe(plain);
  });
});

describe("withReturns / returnKindOf / rowsVisible", () => {
  it("geçişler: tip ↔ SETOF tip adı taşınır; TABLE kolonları; trigger ve void", () => {
    const d = f({ returns: ty("integer") });
    expect(withReturns(d, "setof").returns).toEqual(ty("integer", true));
    expect(withReturns(withReturns(d, "setof"), "type").returns).toEqual(ty("integer"));
    expect(withReturns(d, "table").returns).toEqual({ kind: "table", columns: [] });
    expect(withReturns(d, "trigger").returns).toEqual({ kind: "trigger" });
    expect(withReturns(d, "void").returns).toEqual({ kind: "void" });
    expect(withReturns(f(), "type").returns).toEqual(ty(""));
  });
  it("trigger'a geçişte argüman listesi korunur (arka uç hata verir)", () => {
    const d = f({ args: [arg("in", "a", "integer")] });
    expect(withReturns(d, "trigger").args).toEqual(d.args);
  });
  it("TABLE'a geçişte OUT/INOUT argüman uyarısı", () => {
    const d = withReturns(f({ args: [arg("out", "o", "integer")] }), "table");
    expect(d.args).toHaveLength(1);
    expect(tableOutArgsWarning(d)).toBe(true);
    expect(tableOutArgsWarning(withReturns(f({ args: [arg("in", "a", "integer")] }), "table"))).toBe(false);
    expect(tableOutArgsWarning(f({ args: [arg("out", "o", "integer")] }))).toBe(false);
  });
  it("küme dönmeyen türe geçişte ROWS temizlenir; küme dönende korunur", () => {
    const d = f({ returns: ty("integer", true), rows: "10" });
    expect(withReturns(d, "type").rows).toBeNull();
    expect(withReturns(d, "table").rows).toBe("10");
    expect(withReturns(d, "void").rows).toBeNull();
  });
  it("returnKindOf ve rowsVisible", () => {
    expect(returnKindOf(ty("int"))).toBe("type");
    expect(returnKindOf(ty("int", true))).toBe("setof");
    expect(returnKindOf({ kind: "table", columns: [] })).toBe("table");
    expect(returnKindOf({ kind: "trigger" })).toBe("trigger");
    expect(returnKindOf({ kind: "void" })).toBe("void");
    expect(returnKindOf(null)).toBeNull();
    expect(rowsVisible(f({ returns: ty("int", true) }))).toBe(true);
    expect(rowsVisible(f({ returns: { kind: "table", columns: [] } }))).toBe(true);
    expect(rowsVisible(f({ returns: ty("int") }))).toBe(false);
    expect(rowsVisible(f({ returns: null }))).toBe(false);
  });
  it("returnTypeName / returnColumns", () => {
    expect(returnTypeName(ty("int", true))).toBe("int");
    expect(returnTypeName({ kind: "void" })).toBe("");
    expect(returnColumns({ kind: "table", columns: [{ name: "a", typeName: "int" }] })).toEqual([{ name: "a", typeName: "int" }]);
    expect(returnColumns(null)).toEqual([]);
  });
});

describe("argModes / securityDefinerWarning / recreateLoss", () => {
  it("function ve procedure modları", () => {
    expect(argModes("function")).toEqual(["in", "out", "inout", "variadic"]);
    expect(argModes("procedure")).toEqual(["in", "out", "inout", "variadic"]);
  });
  it("securityDefinerWarning: false → true (create'te true)", () => {
    expect(securityDefinerWarning(f(), f({ securityDefiner: true }))).toBe(true);
    expect(securityDefinerWarning(null, f({ securityDefiner: true }))).toBe(true);
    expect(securityDefinerWarning(f({ securityDefiner: true }), f({ securityDefiner: true }))).toBe(false);
    expect(securityDefinerWarning(f({ securityDefiner: true }), f())).toBe(false);
    expect(securityDefinerWarning(null, f())).toBe(false);
  });
  it("recreateLoss: GRANT yetkileri ve bağımlılar", () => {
    expect(recreateLoss()).toEqual(["GRANT yetkileri", "bağımlı trigger/view'lar (varsa uygulama reddedilir)"]);
  });
});

describe("hasRecreateError / languageChoices", () => {
  const issues = (entries: Array<[string, string]>) => ({ byPath: new Map(entries.map(([p, m]) => [p, [m]])), general: [], generalPaths: [] });
  const MSG = "Bu değişiklik fonksiyonu yeniden oluşturmayı gerektirir; 'Yeniden oluştur' seçin.";
  it("args/returns yollarındaki 'yeniden oluşturmayı gerektirir' hatası", () => {
    expect(hasRecreateError(issues([["args[0].typeName", MSG]]))).toBe(true);
    expect(hasRecreateError(issues([["returns", MSG]]))).toBe(true);
    expect(hasRecreateError(issues([["args", MSG]]))).toBe(true);
    expect(hasRecreateError(issues([["args[0].typeName", "Geçersiz tip."]]))).toBe(false);
    expect(hasRecreateError(issues([["body", MSG]]))).toBe(false);
    expect(hasRecreateError(issues([]))).toBe(false);
  });
  it("languageChoices: mevcut dil listede yoksa başa eklenir", () => {
    const items = [{ value: "plpgsql", detail: "güvenilir" }, { value: "sql", detail: "güvenilir" }];
    expect(languageChoices(items, "plpgsql").map((i) => i.value)).toEqual(["plpgsql", "sql"]);
    expect(languageChoices(items, "plpython3u").map((i) => i.value)).toEqual(["plpython3u", "plpgsql", "sql"]);
    expect(languageChoices([], "")).toEqual([]);
  });
});

describe("review düzeltmeleri", () => {
  it("shouldFocusRecreate: yalnız yükselen kenar (false/undefined → true)", () => {
    expect(shouldFocusRecreate(true, undefined)).toBe(true);
    expect(shouldFocusRecreate(true, false)).toBe(true);
    expect(shouldFocusRecreate(true, true)).toBe(false);
    expect(shouldFocusRecreate(false, true)).toBe(false);
    expect(shouldFocusRecreate(false, undefined)).toBe(false);
  });
  it("stashReturn + withReturns yedeği: TABLE→tip→TABLE ve tip→void→tip veri kaybetmez", () => {
    const cols = [{ name: "a", typeName: "int" }];
    let stash = stashReturn({ typeName: "", columns: [] }, { kind: "table", columns: cols });
    let d = withReturns(f({ returns: { kind: "table", columns: cols } }), "type", stash);
    expect(d.returns).toEqual(ty(""));
    stash = stashReturn(stash, d.returns);
    expect(withReturns(d, "table", stash).returns).toEqual({ kind: "table", columns: cols });
    stash = stashReturn({ typeName: "", columns: [] }, ty("numeric", true));
    d = withReturns(f({ returns: ty("numeric", true) }), "void", stash);
    stash = stashReturn(stash, d.returns);
    expect(withReturns(d, "type", stash).returns).toEqual(ty("numeric"));
    // Yedeksiz eski davranış
    expect(withReturns(f(), "table").returns).toEqual({ kind: "table", columns: [] });
  });
  it("stashReturn: void/trigger/null yedeği değiştirmez", () => {
    const st = { typeName: "x", columns: [] };
    expect(stashReturn(st, { kind: "void" })).toBe(st);
    expect(stashReturn(st, null)).toBe(st);
  });
  it("procedureHiddenErrors: procedure'de gösterilmeyen seçenek/dönüş yollarının hataları", () => {
    const issues = { byPath: new Map([["volatility", ["v"]], ["rows", ["r"]], ["returns.typeName", ["t"]], ["name", ["n"]]]), general: [], generalPaths: [] };
    expect(procedureHiddenErrors("procedure", issues)).toEqual(["v", "r", "t"]);
    expect(procedureHiddenErrors("function", issues)).toEqual([]);
  });
});
