import { describe, expect, it } from "vitest";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Risk } from "../../ipc/bindings/Risk";
import type { Statement } from "../../ipc/bindings/Statement";
import {
  addColumn, addInclude, applyGate, applyOptionsFor, canSubmit, cloneDef, confirmFlags, defName, defsEqual, groupIssues,
  isSafeIntegerString, issuesFor, kindLabel, moveColumn, removeColumn, removeInclude, scriptWithRanges, updateColumn,
  type ApplyGate, type ConfirmState, type GateInput,
} from "./designer-model";

const fe = (path: string, message = path + " hatası"): FieldError => ({ path, message });
const stmt = (sql: string, risk: Risk = "safe"): Statement => ({ sql, risk, transactional: true, note: null });
const view = (p: Partial<PlanView> = {}): PlanView => ({
  statements: [stmt("CREATE SCHEMA a")], warnings: [], errors: [], hash: "h1", risk: "safe", empty: false, ...p,
});
const gin = (p: Partial<GateInput> = {}): GateInput => ({
  locked: false, production: false, serverName: "prod-1", view: view(), viewIsCurrent: true, pending: false, applying: false,
  connected: true, ...p,
});
const col = (name: string, p: Partial<IndexColumn> = {}): IndexColumn => ({
  key: { kind: "column", name }, opclass: null, collation: null, descending: false, nullsFirst: null, ...p,
});
const idx = (p: Partial<IndexDef> = {}): IndexDef => ({
  schema: "public", table: "t", name: "ix", method: "btree", unique: false, columns: [col("a"), col("b"), col("c")], include: ["x", "y"],
  whereClause: null, tablespace: null, fillfactor: null, comment: null, constraint: null, valid: true, ...p,
});

describe("kindLabel / defName", () => {
  it("tür etiketleri PostgreSQL terimleri", () => {
    expect(kindLabel("schema")).toBe("Schema");
    expect(kindLabel("sequence")).toBe("Sequence");
    expect(kindLabel("extension")).toBe("Extension");
    expect(kindLabel("index")).toBe("Index");
  });
  it("defName: şema/eklenti adı; sequence ve indeks şema nitelikli; boş ad boş döner", () => {
    expect(defName({ kind: "schema", name: "app", owner: null, comment: null })).toBe("app");
    expect(defName({ kind: "extension", name: "hstore", schema: null, version: null, cascade: false })).toBe("hstore");
    expect(defName({ kind: "index", ...idx() })).toBe("public.ix");
    expect(defName({ kind: "index", ...idx({ name: "" }) })).toBe("");
    expect(defName({ kind: "schema", name: "", owner: null, comment: null })).toBe("");
    expect(defName({
      kind: "sequence", schema: "s", name: "q", owner: null, dataType: "bigint", increment: "1", minValue: null, maxValue: null,
      start: "1", cache: "1", cycle: false, ownedBy: null, comment: null,
    })).toBe("s.q");
  });
});

describe("cloneDef / defsEqual", () => {
  const def = (): ObjectDef => ({ kind: "index", ...idx() });
  it("derin kopya bağımsız", () => {
    const a = def();
    const b = cloneDef(a);
    expect(b).toEqual(a);
    expect(b).not.toBe(a);
    if (a.kind !== "index" || b.kind !== "index") throw new Error();
    expect(b.columns).not.toBe(a.columns);
    expect(b.columns[0]).not.toBe(a.columns[0]);
    b.columns[0]!.descending = true;
    b.include.push("z");
    expect(a.columns[0]!.descending).toBe(false);
    expect(a.include).toEqual(["x", "y"]);
  });
  it("defsEqual: aynı → true, alan değişince false, alan sırasından bağımsız", () => {
    const a = def();
    expect(defsEqual(a, cloneDef(a))).toBe(true);
    const rev = Object.fromEntries(Object.entries(a).reverse()) as ObjectDef;
    expect(Object.keys(rev)[0]).not.toBe(Object.keys(a)[0]);
    expect(defsEqual(a, rev)).toBe(true);
    const c = cloneDef(a);
    if (c.kind !== "index") throw new Error();
    c.columns[1]!.key = { kind: "column", name: "zz" };
    expect(defsEqual(a, c)).toBe(false);
  });
  it("defsEqual: iç içe nesnelerde alan sırası önemsiz", () => {
    const a = def();
    const b = cloneDef(a);
    if (b.kind !== "index") throw new Error();
    b.columns = b.columns.map((c) => Object.fromEntries(Object.entries(c).reverse()) as IndexColumn);
    expect(defsEqual(a, b)).toBe(true);
  });
  it("defsEqual: dizi sırası önemlidir, null ≠ boş dizge, tür farkı eşit değil", () => {
    const a = def(), b = cloneDef(a);
    if (b.kind !== "index") throw new Error();
    b.include = ["y", "x"];
    expect(defsEqual(a, b)).toBe(false);
    const s1: ObjectDef = { kind: "schema", name: "a", owner: null, comment: null };
    const s2: ObjectDef = { kind: "schema", name: "a", owner: null, comment: "" };
    expect(defsEqual(s1, s2)).toBe(false);
    expect(defsEqual(s1, { kind: "extension", name: "a", schema: null, version: null, cascade: false })).toBe(false);
  });
});

describe("groupIssues / issuesFor", () => {
  it("groupIssues: columns[1].key alan yoluna, old.* ve bilinmeyenler general'a", () => {
    const known = new Set(["name", "columns[1].key", "columns[0].key", "include"]);
    const g = groupIssues(
      [fe("name", "a"), fe("columns[1].key", "b"), fe("old.name", "c"), fe("bilinmeyen", "d"), fe("columns", "e"), fe("name", "f")],
      known,
    );
    expect(g.byPath.get("name")).toEqual(["a", "f"]);
    expect(g.byPath.get("columns[1].key")).toEqual(["b"]);
    expect(g.general).toEqual(["c", "d", "e"]);
    expect(g.byPath.has("old.name")).toBe(false);
    expect(g.byPath.has("columns")).toBe(false);
  });
  it("groupIssues: knownPaths yoksa yalnızca old.* ve üst-düzey 'columns' general", () => {
    const g = groupIssues([fe("anything", "x"), fe("columns", "y"), fe("old.columns[0].key", "z")]);
    expect(g.byPath.get("anything")).toEqual(["x"]);
    expect(g.general).toEqual(["y", "z"]);
  });
  it("groupIssues: bilinen yolun alt yolları (indisli/noktalı) bilinen sayılır; önek yanılgısı olmaz", () => {
    const known = new Set(["columns[0]", "ownedBy", "name"]);
    const g = groupIssues([fe("columns[0].opclass", "a"), fe("ownedBy.table", "b"), fe("nameX", "c"), fe("columns[0]", "d")], known);
    expect(g.byPath.get("columns[0].opclass")).toEqual(["a"]);
    expect(g.byPath.get("ownedBy.table")).toEqual(["b"]);
    expect(g.byPath.get("columns[0]")).toEqual(["d"]);
    expect(g.general).toEqual(["c"]);
  });
  it("groupIssues: boş giriş; girdiyi değiştirmez", () => {
    expect(groupIssues([])).toEqual({ byPath: new Map(), general: [] });
    const input = [fe("name")];
    groupIssues(input, new Set());
    expect(input).toEqual([fe("name")]);
  });
  it("issuesFor: 'columns' isteği columns[0].key hatalarını da kapsar; 'name' yalnızca name", () => {
    const g = groupIssues([
      fe("columns[0].key", "k0"), fe("columns[1].opclass", "o1"), fe("name", "n"), fe("nameplate", "np"), fe("include[0]", "i0"),
    ]);
    expect(issuesFor(g, "columns")).toEqual(["k0", "o1"]);
    expect(issuesFor(g, "columns[1]")).toEqual(["o1"]);
    expect(issuesFor(g, "columns[1].opclass")).toEqual(["o1"]);
    expect(issuesFor(g, "columns[10]")).toEqual([]);
    expect(issuesFor(g, "name")).toEqual(["n"]);
    expect(issuesFor(g, "include")).toEqual(["i0"]);
    expect(issuesFor(g, "yok")).toEqual([]);
  });
});

describe("applyGate", () => {
  const blocked = (reason: string): ApplyGate => ({ kind: "blocked", reason } as ApplyGate);
  it("temel: hatasız, boş olmayan, güncel, güvenli plan → ready", () => {
    expect(applyGate(gin())).toEqual({ kind: "ready" });
  });
  it("kilitli → blocked locked; her şeyden önce gelir", () => {
    expect(applyGate(gin({ locked: true }))).toEqual(blocked("locked"));
    expect(applyGate(gin({
      locked: true, connected: false, applying: true, pending: true, viewIsCurrent: false, view: null,
    }))).toEqual(blocked("locked"));
  });
  it("kilitli değilken: bağlı değil → disconnected (uçuş/bekleme/bayat/hata hepsinden önce)", () => {
    expect(applyGate(gin({ connected: false }))).toEqual(blocked("disconnected"));
    expect(applyGate(gin({
      connected: false, applying: true, pending: true, viewIsCurrent: false, view: view({ errors: [fe("name")], empty: true }),
    }))).toEqual(blocked("disconnected"));
  });
  it("uygulama uçuşta → busy (pending/stale/invalid'den önce)", () => {
    expect(applyGate(gin({ applying: true }))).toEqual(blocked("busy"));
    expect(applyGate(gin({ applying: true, pending: true, viewIsCurrent: false, view: null }))).toEqual(blocked("busy"));
  });
  it("önizleme bekliyor → pending (stale/invalid'den önce)", () => {
    expect(applyGate(gin({ pending: true }))).toEqual(blocked("pending"));
    expect(applyGate(gin({ pending: true, viewIsCurrent: false, view: view({ errors: [fe("name")] }) }))).toEqual(blocked("pending"));
  });
  it("viewIsCurrent=false veya view yok → stale; bayat görünümün hata/boş durumu dikkate alınmaz", () => {
    expect(applyGate(gin({ viewIsCurrent: false }))).toEqual(blocked("stale"));
    expect(applyGate(gin({ view: null }))).toEqual(blocked("stale"));
    expect(applyGate(gin({ viewIsCurrent: false, view: view({ errors: [fe("name")], empty: true }) }))).toEqual(blocked("stale"));
  });
  it("errors var → invalid (empty=true olsa bile invalid; empty ile karıştırılmaz)", () => {
    expect(applyGate(gin({ view: view({ errors: [fe("name")] }) }))).toEqual(blocked("invalid"));
    expect(applyGate(gin({ view: view({ errors: [fe("name")], empty: true, statements: [] }) }))).toEqual(blocked("invalid"));
  });
  it("errors yok + empty → empty (boş plan asla uygulanmaz, risk ne olursa olsun)", () => {
    expect(applyGate(gin({ view: view({ empty: true, statements: [] }) }))).toEqual(blocked("empty"));
    expect(applyGate(gin({ view: view({ empty: true, statements: [], risk: "data_loss" }), production: true }))).toEqual(blocked("empty"));
  });
  it("uyarılar (warnings) uygulamayı engellemez", () => {
    expect(applyGate(gin({ view: view({ warnings: ["dikkat"] }) }))).toEqual({ kind: "ready" });
  });
  it("risk: safe → ready; destructive → needsConfirm (üretimde de)", () => {
    expect(applyGate(gin({ view: view({ risk: "destructive" }) }))).toEqual({ kind: "needsConfirm" });
    expect(applyGate(gin({ view: view({ risk: "destructive" }), production: true }))).toEqual({ kind: "needsConfirm" });
    expect(applyGate(gin({ view: view({ risk: "safe" }), production: true }))).toEqual({ kind: "ready" });
  });
  it("data_loss + production → needsProductionName(expected=serverName); production değil → needsConfirm", () => {
    expect(applyGate(gin({ view: view({ risk: "data_loss" }), production: true, serverName: "Üretim DB" })))
      .toEqual({ kind: "needsProductionName", expected: "Üretim DB" });
    expect(applyGate(gin({ view: view({ risk: "data_loss" }), production: false }))).toEqual({ kind: "needsConfirm" });
  });
  it("engeller risk onayından önce gelir (kilitli/bekleyen + data_loss)", () => {
    expect(applyGate(gin({ view: view({ risk: "data_loss" }), production: true, locked: true }))).toEqual(blocked("locked"));
    expect(applyGate(gin({ view: view({ risk: "data_loss" }), production: true, pending: true }))).toEqual(blocked("pending"));
  });
  it("girdiyi değiştirmez", () => {
    const i = gin();
    const snap = JSON.stringify(i);
    applyGate(i);
    expect(JSON.stringify(i)).toBe(snap);
  });
});

describe("canSubmit / confirmFlags", () => {
  const cs = (checked: boolean, typedName = ""): ConfirmState => ({ checked, typedName });
  const ready: ApplyGate = { kind: "ready" };
  const conf: ApplyGate = { kind: "needsConfirm" };
  const prod: ApplyGate = { kind: "needsProductionName", expected: "Prod-1" };
  const blockedReasons = ["locked", "invalid", "empty", "stale", "pending", "busy", "disconnected"] as const;

  it("ready → true; onay durumundan bağımsız; bayraklar false", () => {
    expect(canSubmit(ready, cs(false))).toBe(true);
    expect(canSubmit(ready, cs(true, "x"))).toBe(true);
    expect(confirmFlags(ready, cs(true, "Prod-1"))).toEqual({ confirmed: false, confirmedProduction: false });
  });
  it("blocked → hiçbir zaman gönderilemez (tam onaylı olsa da); bayraklar false", () => {
    for (const reason of blockedReasons) {
      const g: ApplyGate = { kind: "blocked", reason };
      expect(canSubmit(g, cs(true, "Prod-1"))).toBe(false);
      expect(confirmFlags(g, cs(true, "Prod-1"))).toEqual({ confirmed: false, confirmedProduction: false });
    }
  });
  it("needsConfirm: onay kutusu olmadan gönderilemez; işaretlenince confirmed=true, confirmedProduction=false", () => {
    expect(canSubmit(conf, cs(false))).toBe(false);
    expect(canSubmit(conf, cs(true))).toBe(true);
    expect(canSubmit(conf, cs(false, "Prod-1"))).toBe(false);
    expect(confirmFlags(conf, cs(false))).toEqual({ confirmed: false, confirmedProduction: false });
    expect(confirmFlags(conf, cs(true, "Prod-1"))).toEqual({ confirmed: true, confirmedProduction: false });
  });
  it("needsProductionName: kutu VE tam ad gerekir; büyük/küçük harf duyarlı; kenar boşlukları kırpılır", () => {
    expect(canSubmit(prod, cs(false, "Prod-1"))).toBe(false);
    expect(canSubmit(prod, cs(true, ""))).toBe(false);
    expect(canSubmit(prod, cs(true, "prod-1"))).toBe(false);
    expect(canSubmit(prod, cs(true, "Prod-"))).toBe(false);
    expect(canSubmit(prod, cs(true, "Prod-1x"))).toBe(false);
    expect(canSubmit(prod, cs(true, "Prod -1"))).toBe(false);
    expect(canSubmit(prod, cs(true, "Prod-1"))).toBe(true);
    expect(canSubmit(prod, cs(true, "  Prod-1\t\n"))).toBe(true);
  });
  it("needsProductionName: boş beklenen ad hiçbir zaman eşleşmiş sayılmaz", () => {
    const g: ApplyGate = { kind: "needsProductionName", expected: "" };
    expect(canSubmit(g, cs(true, ""))).toBe(false);
    expect(canSubmit(g, cs(true, "   "))).toBe(false);
    expect(confirmFlags(g, cs(true, "")).confirmedProduction).toBe(false);
  });
  it("confirmFlags: confirmedProduction yalnızca needsProductionName'de ve ad eşleşirse true", () => {
    expect(confirmFlags(prod, cs(true, "Prod-1"))).toEqual({ confirmed: true, confirmedProduction: true });
    expect(confirmFlags(prod, cs(true, " Prod-1 "))).toEqual({ confirmed: true, confirmedProduction: true });
    expect(confirmFlags(prod, cs(true, "prod-1"))).toEqual({ confirmed: true, confirmedProduction: false });
    expect(confirmFlags(prod, cs(false, "Prod-1"))).toEqual({ confirmed: false, confirmedProduction: false });
    expect(confirmFlags(prod, cs(true, ""))).toEqual({ confirmed: true, confirmedProduction: false });
  });
  it("tutarlılık: canSubmit true iken needs* kapılarında bayraklar sunucunun beklediğini taşır", () => {
    for (const g of [conf, prod]) {
      const c = cs(true, "Prod-1");
      expect(canSubmit(g, c)).toBe(true);
      const f = confirmFlags(g, c);
      expect(f.confirmed).toBe(true);
      expect(f.confirmedProduction).toBe(g.kind === "needsProductionName");
    }
  });
});

describe("applyOptionsFor", () => {
  it("applyId, lockTimeoutMs ve görünümün hash'i taşınır; bayraklar kapıdan", () => {
    const v = view({ hash: "abc", risk: "data_loss" });
    const g: ApplyGate = { kind: "needsProductionName", expected: "P" };
    expect(applyOptionsFor("id-1", 5000, v, g, { checked: true, typedName: "P" })).toEqual({
      applyId: "id-1", lockTimeoutMs: 5000, expectedHash: "abc", confirmed: true, confirmedProduction: true,
    });
    expect(applyOptionsFor("id-2", 0, view({ hash: "z" }), { kind: "ready" }, { checked: true, typedName: "P" })).toEqual({
      applyId: "id-2", lockTimeoutMs: 0, expectedHash: "z", confirmed: false, confirmedProduction: false,
    });
  });
});

describe("scriptWithRanges", () => {
  it("aralıklar ifade metnini tam kapsar; ';' ve boş satır ayırıcılar aralık dışı", () => {
    const sts = [stmt("CREATE SCHEMA a"), stmt("COMMENT ON SCHEMA a IS 'x'"), stmt("DROP SCHEMA b")];
    const { text, ranges } = scriptWithRanges(sts);
    expect(text).toBe("CREATE SCHEMA a;\n\nCOMMENT ON SCHEMA a IS 'x';\n\nDROP SCHEMA b;");
    expect(ranges).toHaveLength(3);
    ranges.forEach(([s, e], i) => {
      expect(text.slice(s, e)).toBe(sts[i]!.sql);
      expect(text[e]).toBe(";");
    });
    expect(text.slice(ranges[0]![1], ranges[1]![0])).toBe(";\n\n");
  });
  it("tek ifade, boş liste, çok satırlı ifade", () => {
    expect(scriptWithRanges([])).toEqual({ text: "", ranges: [] });
    expect(scriptWithRanges([stmt("X")])).toEqual({ text: "X;", ranges: [[0, 1]] });
    const m = scriptWithRanges([stmt("A\nB"), stmt("C")]);
    expect(m.ranges.map(([s, e]) => m.text.slice(s, e))).toEqual(["A\nB", "C"]);
  });
  it("çok baytlı karakterlerde aralıklar UTF-16 indeksidir (string.slice ile tutarlı)", () => {
    const m = scriptWithRanges([stmt("COMMENT IS 'ğüş😀'"), stmt("B")]);
    expect(m.ranges.map(([s, e]) => m.text.slice(s, e))).toEqual(["COMMENT IS 'ğüş😀'", "B"]);
  });
});

describe("isSafeIntegerString", () => {
  it("geçerli: '0','+5','-0','007','-1', i64 sınırları", () => {
    for (const s of ["0", "+5", "-0", "007", "-1", "9223372036854775807", "-9223372036854775808", "+0009223372036854775807"]) {
      expect(isSafeIntegerString(s), s).toBe(true);
    }
  });
  it("geçersiz: '', boşluklu, '1e3', ondalık, işaret tek başına, i64 dışı, onaltılık", () => {
    for (const s of ["", " 5", "5 ", "1e3", "1.5", "+", "-", "--1", "+-1", "0x10", "abc", "٣", "9223372036854775808", "-9223372036854775809", "99999999999999999999"]) {
      expect(isSafeIntegerString(s), s).toBe(false);
    }
  });
});

describe("indeks kolon işlemleri", () => {
  const names = (d: IndexDef) => d.columns.map((c) => (c.key.kind === "column" ? c.key.name : "<expr>"));
  it("addColumn sona ekler, girdiyi değiştirmez", () => {
    const d = idx();
    const n = addColumn(d, col("z"));
    expect(names(n)).toEqual(["a", "b", "c", "z"]);
    expect(names(d)).toEqual(["a", "b", "c"]);
    expect(n).not.toBe(d);
  });
  it("removeColumn: ortadan siler; sınır dışı → aynı nesne", () => {
    const d = idx();
    expect(names(removeColumn(d, 1))).toEqual(["a", "c"]);
    expect(names(d)).toEqual(["a", "b", "c"]);
    expect(removeColumn(d, -1)).toBe(d);
    expect(removeColumn(d, 3)).toBe(d);
    expect(removeColumn(d, 1.5)).toBe(d);
  });
  it("moveColumn: ileri/geri taşır; aynı yer ve sınır dışı → aynı nesne", () => {
    const d = idx();
    expect(names(moveColumn(d, 0, 2))).toEqual(["b", "c", "a"]);
    expect(names(moveColumn(d, 2, 0))).toEqual(["c", "a", "b"]);
    expect(names(moveColumn(d, 0, 1))).toEqual(["b", "a", "c"]);
    expect(names(d)).toEqual(["a", "b", "c"]);
    expect(moveColumn(d, 1, 1)).toBe(d);
    expect(moveColumn(d, -1, 1)).toBe(d);
    expect(moveColumn(d, 0, 3)).toBe(d);
    expect(moveColumn(d, 3, 0)).toBe(d);
  });
  it("updateColumn: yalnızca ilgili kolonu yamalar; diğerleri aynı referans; sınır dışı → aynı nesne", () => {
    const d = idx();
    const n = updateColumn(d, 1, { descending: true, collation: "C" });
    expect(n.columns[1]).toEqual(col("b", { descending: true, collation: "C" }));
    expect(n.columns[0]).toBe(d.columns[0]);
    expect(n.columns[2]).toBe(d.columns[2]);
    expect(d.columns[1]!.descending).toBe(false);
    expect(updateColumn(d, 5, { descending: true })).toBe(d);
    expect(updateColumn(d, -1, { descending: true })).toBe(d);
    const e = updateColumn(d, 0, { key: { kind: "expression", sql: "lower(a)" }, nullsFirst: true });
    expect(e.columns[0]!.key).toEqual({ kind: "expression", sql: "lower(a)" });
    expect(e.columns[0]!.nullsFirst).toBe(true);
  });
  it("addInclude/removeInclude değişmez; sınır dışı → aynı nesne; diğer alanlar korunur", () => {
    const d = idx();
    const n = addInclude(d, "w");
    expect(n.include).toEqual(["x", "y", "w"]);
    expect(d.include).toEqual(["x", "y"]);
    expect(n.columns).toBe(d.columns);
    expect(removeInclude(d, 0).include).toEqual(["y"]);
    expect(d.include).toEqual(["x", "y"]);
    expect(removeInclude(d, 2)).toBe(d);
    expect(removeInclude(d, -1)).toBe(d);
  });
});
