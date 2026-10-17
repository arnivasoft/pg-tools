import { describe, expect, it } from "vitest";
import type { AclDef } from "../../ipc/bindings/AclDef";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Risk } from "../../ipc/bindings/Risk";
import type { SequenceDef } from "../../ipc/bindings/SequenceDef";
import type { Statement } from "../../ipc/bindings/Statement";
import {
  addColumn, addInclude, focusFieldOf, parseFocusField, applyGate, applyOptionsFor, canSubmit, cloneDef, confirmFlags, defName, defsEqual, groupIssues,
  isSafeIntegerString, issuesFor, kindLabel, localBlock, moveColumn, previewEmptyState, removeColumn, removeInclude, scriptWithRanges,
  statementRangesToLineRanges, updateColumn, isServerScoped, planHasRedacted, scriptActionsEnabled, designerTabKey,
  aclTabKey, defaultAclTabKey, isObjectDef,
  type ApplyGate, type ConfirmState, type GateInput,
} from "./designer-model";

const fe = (path: string, message = path + " hatası"): FieldError => ({ path, message });
const stmt = (sql: string, risk: Risk = "safe"): Statement => ({ sql, risk, transactional: true, note: null, redacted: false });
const view = (p: Partial<PlanView> = {}): PlanView => ({
  statements: [stmt("CREATE SCHEMA a")], warnings: [], errors: [], hash: "h1", risk: "safe", empty: false, ...p,
});
const gin = (p: Partial<GateInput> = {}): GateInput => ({
  locked: false, production: false, serverName: "prod-1", view: view(), viewIsCurrent: true, pending: false, applying: false,
  connected: true, loading: false, secretMissing: false, ...p,
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
  it("Plan 18 türleri: etiketler; function/procedure/type/domain şema nitelikli, trigger tablo bağlamıyla", () => {
    expect(["function", "procedure", "trigger", "type", "domain"].map((k) => kindLabel(k as ObjectKind))).toEqual([
      "Function", "Procedure", "Trigger", "Type", "Domain",
    ]);
    const base = { schema: "s", name: "n", owner: null, comment: null, editable: true, unsupported: [] };
    const fn = {
      ...base, language: "sql", args: [], body: "SELECT 1", volatility: "volatile" as const, strict: false, securityDefiner: false,
      leakproof: false, parallel: "unsafe" as const, cost: null, rows: null, settings: [], recreate: false,
    };
    expect(defName({ kind: "function", ...fn, returns: { kind: "void" } })).toBe("s.n");
    expect(defName({ kind: "procedure", ...fn, returns: null })).toBe("s.n");
    expect(defName({ kind: "type", ...base, body: { variant: "enum", values: [] } })).toBe("s.n");
    expect(defName({
      kind: "domain", ...base, baseType: "integer", collation: null, default: null, notNull: false, constraints: [],
    })).toBe("s.n");
    const trg = {
      schema: "s", table: "t", name: "tr", timing: "before" as const, level: "row" as const, when: null,
      events: { insert: true, update: false, updateColumns: [], delete: false, truncate: false },
      functionSchema: "s", functionName: "tf", args: [], oldTable: null, newTable: null, constraint: false, deferrable: false,
      initiallyDeferred: false, enabled: "origin" as const, comment: null, editable: true, unsupported: [],
    };
    expect(defName({ kind: "trigger", ...trg })).toBe("s.t.tr");
    expect(defName({ kind: "trigger", ...trg, name: "" })).toBe("");
  });
  it("Plan 14 türleri: etiketler; view/MV şema nitelikli, database/role düz ad", () => {
    expect([kindLabel("view"), kindLabel("materialized_view"), kindLabel("database"), kindLabel("role")]).toEqual([
      "View", "Materialized View", "Database", "Role",
    ]);
    const q = { owner: null, query: "SELECT 1", comment: null, columns: [], unsupported: [] };
    expect(defName({
      kind: "view", schema: "s", name: "v", checkOption: null, securityBarrier: false, securityInvoker: false, recreate: false, ...q,
    })).toBe("s.v");
    expect(defName({
      kind: "materialized_view", schema: "s", name: "m", tablespace: null, fillfactor: null, withData: true, refresh: null, ...q,
    })).toBe("s.m");
    expect(defName({
      kind: "database", name: "d", owner: null, template: null, encoding: null, localeProvider: null, lcCollate: null, lcCtype: null,
      providerLocale: null, tablespace: null, connectionLimit: -1, isTemplate: false, allowConnections: true, settings: [], comment: null,
    })).toBe("d");
    expect(defName({
      kind: "role", name: "r", login: false, superuser: false, createdb: false, createrole: false, inherit: true, replication: false,
      bypassrls: false, connectionLimit: -1, validUntil: null, password: "keep", memberOf: [], settings: [], comment: null, unsupported: [],
    })).toBe("r");
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
    expect(groupIssues([])).toEqual({ byPath: new Map(), general: [], generalPaths: [] });
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
  it("yükleniyor/yükleme hatası → loading (disconnected'dan sonra, busy/pending/stale'den önce)", () => {
    expect(applyGate(gin({ loading: true }))).toEqual(blocked("loading"));
    expect(applyGate(gin({ loading: true, applying: true, pending: true, view: null }))).toEqual(blocked("loading"));
    expect(applyGate(gin({ loading: true, connected: false }))).toEqual(blocked("disconnected"));
    expect(applyGate(gin({ loading: true, locked: true }))).toEqual(blocked("locked"));
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
  const blockedReasons = ["locked", "invalid", "empty", "stale", "pending", "busy", "disconnected", "loading"] as const;

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
      applyId: "id-1", lockTimeoutMs: 5000, expectedHash: "abc", confirmed: true, confirmedProduction: true, password: null,
    });
    expect(applyOptionsFor("id-2", 0, view({ hash: "z" }), { kind: "ready" }, { checked: true, typedName: "P" })).toEqual({
      applyId: "id-2", lockTimeoutMs: 0, expectedHash: "z", confirmed: false, confirmedProduction: false, password: null,
    });
  });

  it("canSubmit değilse null (gönderilemez): blocked, kutusuz onay, yanlış ad, boş beklenen ad", () => {
    const v = view({ risk: "data_loss" });
    const off: ConfirmState = { checked: false, typedName: "" };
    expect(applyOptionsFor("a", 5000, v, { kind: "blocked", reason: "stale" }, { checked: true, typedName: "P" })).toBeNull();
    expect(applyOptionsFor("a", 5000, v, { kind: "needsConfirm" }, off)).toBeNull();
    expect(applyOptionsFor("a", 5000, v, { kind: "needsProductionName", expected: "P" }, { checked: true, typedName: "p" })).toBeNull();
    expect(applyOptionsFor("a", 5000, v, { kind: "needsProductionName", expected: "" }, { checked: true, typedName: "" })).toBeNull();
    expect(applyOptionsFor("a", 5000, v, { kind: "needsConfirm" }, { checked: true, typedName: "" })).toMatchObject({ confirmed: true });
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

describe("statementRangesToLineRanges", () => {
  it("birleşik betikte her ifadenin satırları (1 tabanlı, kapsayıcı)", () => {
    const { text, ranges } = scriptWithRanges([stmt("CREATE SCHEMA a"), stmt("A\nB\nC"), stmt("DROP SCHEMA b")]);
    // 1: CREATE…;  2: boş  3-5: A,B,C;  6: boş  7: DROP…;
    expect(statementRangesToLineRanges(text, ranges)).toEqual([{ from: 1, to: 1 }, { from: 3, to: 5 }, { from: 7, to: 7 }]);
  });
  it("sondaki satır sonu sonraki satıra taşmaz; boş ifade ve boş metin", () => {
    expect(statementRangesToLineRanges("A\n;", [[0, 2]])).toEqual([{ from: 1, to: 1 }]);
    expect(statementRangesToLineRanges("A;\n\n;", [[0, 1], [4, 4]])).toEqual([{ from: 1, to: 1 }, { from: 3, to: 3 }]);
    expect(statementRangesToLineRanges("", [])).toEqual([]);
  });
  it("metin dışı aralık son satıra kırpılır", () => {
    expect(statementRangesToLineRanges("A\nB", [[2, 50]])).toEqual([{ from: 2, to: 2 }]);
    expect(statementRangesToLineRanges("A\nB", [[-5, 0]])).toEqual([{ from: 1, to: 1 }]);
  });
});

describe("previewEmptyState", () => {
  it("görünüm yokken: bekliyorsa hazırlanıyor; değilse create → formu tamamlayın, alter → değişiklik yok", () => {
    expect(previewEmptyState(null, true, "create")).toBe("pending");
    expect(previewEmptyState(null, true, "alter")).toBe("pending");
    expect(previewEmptyState(null, false, "create")).toBe("incomplete");
    expect(previewEmptyState(null, false, "alter")).toBe("noChanges");
  });
  it("hatalı görünüm (empty=true da taşır) → formu tamamlayın; boş → değişiklik yok; ifadeli → null", () => {
    expect(previewEmptyState(view({ errors: [fe("name")], empty: true, statements: [] }), false, "alter")).toBe("incomplete");
    expect(previewEmptyState(view({ empty: true, statements: [] }), false, "alter")).toBe("noChanges");
    expect(previewEmptyState(view(), false, "create")).toBeNull();
  });
  it("görünüm yokken yüklenen sekme → yükleniyor; hazır olmayan sekmede (hata/kesik/yok) metin yok", () => {
    expect(previewEmptyState(null, false, "alter", "loading")).toBe("loading");
    expect(previewEmptyState(null, true, "create", "loading")).toBe("loading");
    for (const s of ["error", "disconnected", "gone"] as const) {
      expect(previewEmptyState(null, false, "alter", s)).toBeNull();
      expect(previewEmptyState(null, false, "create", s)).toBeNull();
    }
  });
  it("önizleme hatası + görünüm yok → metin yok; yeniden bekliyorsa hazırlanıyor; görünüm varsa değişmez", () => {
    expect(previewEmptyState(null, false, "alter", "ready", "bağlantı koptu")).toBeNull();
    expect(previewEmptyState(null, false, "create", "ready", "bağlantı koptu")).toBeNull();
    expect(previewEmptyState(null, true, "create", "ready", "eski hata")).toBe("pending");
    expect(previewEmptyState(view(), false, "create", "ready", "hata")).toBeNull();
    expect(previewEmptyState(view({ empty: true, statements: [] }), false, "alter", "ready", "hata")).toBe("noChanges");
  });
  it("bekleyen önizlemede eski görünüm gösterilmeye devam eder", () => {
    expect(previewEmptyState(view(), true, "create")).toBeNull();
    expect(previewEmptyState(view({ empty: true, statements: [] }), true, "alter")).toBe("noChanges");
  });
});

describe("localBlock (yerel engel)", () => {
  const seqDef = (p: Partial<SequenceDef> = {}): ObjectDef => ({
    kind: "sequence", schema: "app", name: "s", owner: null, dataType: "bigint", increment: "1", minValue: null,
    maxValue: null, start: "1", cache: "1", cycle: false, ownedBy: null, comment: null, ...p,
  });
  it("geçerli sequence ve diğer türler → null", () => {
    expect(localBlock(seqDef())).toBeNull();
    expect(localBlock(seqDef({ increment: "-5", minValue: "-100", maxValue: "-1", start: "-1", cache: "+20" }))).toBeNull();
    expect(localBlock(seqDef({ minValue: "-9223372036854775808", maxValue: "9223372036854775807" }))).toBeNull();
    expect(localBlock({ kind: "schema", name: "", owner: null, comment: null })).toBeNull();
    expect(localBlock(null)).toBeNull();
  });
  it.each(["", "abc", "1e3", "-", "9223372036854775808", " 1", "1.0"])("zorunlu sayı %j → engel", (v) => {
    for (const k of ["increment", "start", "cache"] as const) expect(localBlock(seqDef({ [k]: v }))).toBe("sequenceNumbers");
  });
  it("min/max: null serbest, boş dizge ve biçimsiz → engel", () => {
    expect(localBlock(seqDef({ minValue: null, maxValue: null }))).toBeNull();
    expect(localBlock(seqDef({ minValue: "" }))).toBe("sequenceNumbers");
    expect(localBlock(seqDef({ maxValue: "" }))).toBe("sequenceNumbers");
    expect(localBlock(seqDef({ maxValue: "-" }))).toBe("sequenceNumbers");
    expect(localBlock(seqDef({ minValue: "-9223372036854775809" }))).toBe("sequenceNumbers");
  });
});

describe("focusFieldOf / parseFocusField", () => {
  it("biçim ve gidiş-dönüş; bozuk alanlar null", () => {
    expect(focusFieldOf({ kind: "column", id: 3 })).toBe("column:3");
    expect(parseFocusField(focusFieldOf({ kind: "constraint", id: 40 }))).toEqual({ kind: "constraint", id: 40 });
    for (const bad of ["name", "column:", "column:-1", "index:3"]) expect(parseFocusField(bad)).toBeNull();
  });
});

const redactedView = (p: Partial<PlanView> = {}): PlanView =>
  view({ statements: [stmt("CREATE ROLE r LOGIN"), { ...stmt("ALTER ROLE r PASSWORD ********"), redacted: true }], ...p });

describe("Plan 15: sunucu kapsamı, gizlenmiş plan, parola kapısı", () => {
  it("isServerScoped: yalnızca database ve role", () => {
    const table: Record<ObjectKind, boolean> = {
      schema: false, sequence: false, extension: false, index: false, table: false, view: false, materialized_view: false,
      database: true, role: true, function: false, procedure: false, trigger: false, type: false, domain: false,
    };
    for (const [k, v] of Object.entries(table)) expect(isServerScoped(k as ObjectKind)).toBe(v);
  });

  it("planHasRedacted: null → false; gizlenmiş ifade varsa true", () => {
    expect(planHasRedacted(null)).toBe(false);
    expect(planHasRedacted(view())).toBe(false);
    expect(planHasRedacted(redactedView())).toBe(true);
  });

  it("applyGate: secretMissing önceliği empty'den sonra, risk'ten önce", () => {
    // empty + secretMissing → empty
    expect(applyGate(gin({ view: view({ empty: true }), secretMissing: true }))).toEqual({ kind: "blocked", reason: "empty" });
    // invalid + secretMissing → invalid
    expect(applyGate(gin({ view: view({ errors: [fe("password")], empty: true }), secretMissing: true }))).toEqual({ kind: "blocked", reason: "invalid" });
    // stale + secretMissing → stale
    expect(applyGate(gin({ viewIsCurrent: false, secretMissing: true }))).toEqual({ kind: "blocked", reason: "stale" });
    // destructive / data_loss (üretim) + secretMissing → password
    expect(applyGate(gin({ view: redactedView({ risk: "destructive" }), secretMissing: true }))).toEqual({ kind: "blocked", reason: "password" });
    expect(applyGate(gin({ view: redactedView({ risk: "data_loss" }), production: true, secretMissing: true }))).toEqual({ kind: "blocked", reason: "password" });
    expect(applyGate(gin({ view: redactedView(), secretMissing: true }))).toEqual({ kind: "blocked", reason: "password" });
    // parola hazır → risk kapısı
    expect(applyGate(gin({ view: redactedView({ risk: "destructive" }) }))).toEqual({ kind: "needsConfirm" });
    expect(applyGate(gin({ view: redactedView() }))).toEqual({ kind: "ready" });
  });

  it("applyGate: partial (kısmi create) locked/disconnected'dan sonra, diğer her şeyden önce", () => {
    expect(applyGate(gin({ partial: true }))).toEqual({ kind: "blocked", reason: "partial" });
    expect(applyGate(gin({ partial: true, viewIsCurrent: false, pending: true, loading: true }))).toEqual({ kind: "blocked", reason: "partial" });
    expect(applyGate(gin({ partial: true, view: view({ risk: "data_loss" }), production: true }))).toEqual({ kind: "blocked", reason: "partial" });
    expect(applyGate(gin({ partial: true, locked: true }))).toEqual({ kind: "blocked", reason: "locked" });
    expect(applyGate(gin({ partial: true, connected: false }))).toEqual({ kind: "blocked", reason: "disconnected" });
    expect(applyGate(gin({ partial: false }))).toEqual({ kind: "ready" });
  });

  it("applyOptionsFor: gizlenmiş planda parola geçer; gizlenmemiş planda verilen parola null'a çekilir", () => {
    const c = { checked: false, typedName: "" };
    expect(applyOptionsFor("a", 5000, redactedView(), { kind: "ready" }, c, "Gizli-1")).toMatchObject({ password: "Gizli-1" });
    expect(applyOptionsFor("a", 5000, redactedView(), { kind: "ready" }, c, null)).toMatchObject({ password: null });
    expect(applyOptionsFor("a", 5000, view(), { kind: "ready" }, c, "Gizli-1")).toMatchObject({ password: null });
    expect(applyOptionsFor("a", 5000, view(), { kind: "ready" }, c)).toMatchObject({ password: null });
    expect(applyOptionsFor("a", 5000, redactedView(), { kind: "blocked", reason: "password" }, c, "Gizli-1")).toBeNull();
  });

  it("scriptActionsEnabled: gizlenmiş planda copy/open false; gizlenmemiş planda true; null plan → false", () => {
    expect(scriptActionsEnabled(redactedView())).toEqual({ copy: false, open: false });
    expect(scriptActionsEnabled(view())).toEqual({ copy: true, open: true });
    expect(scriptActionsEnabled(null)).toEqual({ copy: false, open: false });
  });

  it("designerTabKey: databaseOid null → alter:role:s1:null:16384; create anahtarında da null", () => {
    expect(designerTabKey([], { serverId: "s1", databaseOid: null, object: { kind: "role", oid: 16384 } })).toBe("alter:role:s1:null:16384");
    expect(designerTabKey([], { serverId: "s1", databaseOid: null, create: { kind: "database", context: { schemaOid: null, tableOid: null }, preset: null } }))
      .toBe("create:database:s1:null:::1");
  });
});

// ---- Plan 22: yetki varyantları ----

const aclDef = (p: Partial<AclDef> = {}): AclDef => ({
  kind: "table", schema: "public", name: "orders", signature: null, owner: "alice", viewer: "alice", explicit: true,
  grants: [{ grantee: null, privileges: [{ privilege: "select", grantOption: false }] }], foreign: [], columns: [],
  editable: true, unsupported: [], available: ["select", "insert"], ...p,
});
const defAclDef = (p: Partial<DefaultAclDef> = {}): DefaultAclDef => ({
  role: "alice", schema: null, viewer: "alice",
  sections: [{ objectType: "tables", grants: [], baseline: [], available: ["select"] }], editable: true, unsupported: [], ...p,
});

describe("Plan 22: yetki tanımları (defName / anahtarlar / eşitlik)", () => {
  it("defName: acl → şema.ad, imza varsa ad(imza); şemasız nesnede yalnız ad", () => {
    expect(defName(aclDef())).toBe("public.orders");
    expect(defName(aclDef({ kind: "function", name: "f", signature: "a integer" }))).toBe("public.f(a integer)");
    expect(defName(aclDef({ kind: "function", name: "g", signature: "" }))).toBe("public.g()");
    expect(defName(aclDef({ kind: "database", schema: null, name: "app" }))).toBe("app");
  });
  it("defName: default_acl → rol (+ / şema)", () => {
    expect(defName(defAclDef())).toBe("alice");
    expect(defName(defAclDef({ schema: "app" }))).toBe("alice / app");
  });
  it("isObjectDef: yalnız nesne tanımları", () => {
    expect(isObjectDef({ kind: "schema", name: "a", owner: null, comment: null })).toBe(true);
    expect(isObjectDef(aclDef())).toBe(false);
    expect(isObjectDef(defAclDef())).toBe(false);
  });
  it("isObjectDef koruması: hiçbir ObjectDef varyantı `viewer` taşımaz (tür düzeyi; varyant kazanırsa typecheck kırılır)", () => {
    // `isObjectDef` yetki tanımlarını `viewer` alanıyla ayırır; bir nesne türü `viewer` alanı kazanırsa bu satır derlenmez.
    type ViewerVariants = Extract<ObjectDef, { viewer: unknown }>;
    const noViewer: [ViewerVariants] extends [never] ? true : false = true;
    expect(noViewer).toBe(true);
  });
  it("cloneDef / defsEqual birleşim tipiyle çalışır", () => {
    const a = aclDef();
    const c = cloneDef(a);
    expect(c).toEqual(a);
    expect(c).not.toBe(a);
    expect(defsEqual(a, c)).toBe(true);
    c.grants[0]!.privileges[0]!.grantOption = true;
    expect(defsEqual(a, c)).toBe(false);
    expect(defsEqual(defAclDef(), cloneDef(defAclDef()))).toBe(true);
  });
  it("localBlock yetki tanımlarında engel üretmez", () => {
    expect(localBlock(aclDef({ kind: "sequence" }))).toBeNull();
    expect(localBlock(defAclDef())).toBeNull();
  });
  it("aclTabKey / defaultAclTabKey", () => {
    expect(aclTabKey("table", "S1", 5, 42)).toBe("acl:table:S1:5:42");
    expect(aclTabKey("database", "S1", null, 7)).toBe("acl:database:S1:null:7");
    expect(defaultAclTabKey("S1", 5, null)).toBe("defacl:S1:5:0");
    expect(defaultAclTabKey("S1", 5, 2200)).toBe("defacl:S1:5:2200");
  });
});
