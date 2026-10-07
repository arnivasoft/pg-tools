import { describe, expect, it } from "vitest";
import type { ConstraintDef } from "../../ipc/bindings/ConstraintDef";
import type { TableDef } from "../../ipc/bindings/TableDef";
import { defsEqual } from "./designer-model";
import {
  addConstraint, addConstraintColumn, addConstraintSnap, canRestoreConstraint, pointFkAtOther, pointFkAtSelf, removeConstraintSnap, restoreConstraintSnap, addFkPair, availableColumns, canAddPrimaryKey, constraintColumns, constraintLabel, danglingColumns,
  newConstraint, notValidMode, originalConstraint, primaryKeyNullable, removeConstraint, removeConstraintColumn, removeFkPair,
  removedConstraints, restoreConstraint, supportsDeferrable, supportsNotValid, switchConstraintType, unsupportedKind, updateConstraint,
  updateFkPair, withDeferrable, withFkActions, withFkTarget, withInitiallyDeferred, withPrimaryKeyNotNull,
} from "./table-constraints";
import type { SelfSnap } from "./table-constraints";
import { newColumn, renameTable, selfRefs } from "./table-model";

const T = (p: Partial<TableDef> = {}): TableDef => ({
  schema: "public", name: "t", owner: null, tablespace: null, persistence: "permanent", columns: [], constraints: [], partitionBy: null,
  fillfactor: null, comment: null, partitionText: null, isPartition: false, inherits: false, unsupported: [], ...p,
});
const col = (name: string, attnum: number | null = null, notNull = false) => ({ ...newColumn(), name, attnum, dataType: "integer", notNull });
const base = { name: null, deferrable: false, initiallyDeferred: false, notValid: false };
const pk = (columns: string[], oid: number | null = null): ConstraintDef => ({ ...base, oid, kind: { type: "primary_key", columns, include: [] } });
const uq = (columns: string[], oid: number | null = null, include: string[] = []): ConstraintDef => ({ ...base, oid, kind: { type: "unique", columns, include, nullsNotDistinct: false } });
const chk = (oid: number | null = null): ConstraintDef => ({ ...base, oid, kind: { type: "check", expr: "a>0", noInherit: false } });
const fk = (columns: string[], refTable: string, refColumns: string[], oid: number | null = null, refSchema = "public"): ConstraintDef => ({
  ...base, oid, kind: { type: "foreign_key", columns, refSchema, refTable, refColumns, matchType: "simple", onUpdate: "no_action", onDelete: "no_action" },
});

describe("kısıt işlemleri", () => {
  it("newConstraint: her tür için alanlar; FK varsayılan hedef şeması taslağın şeması ve tek boş eşleşme", () => {
    const d = T({ schema: "app" });
    const b = { oid: null, name: null, deferrable: false, initiallyDeferred: false, notValid: false };
    expect(newConstraint("primary_key", d)).toEqual({ ...b, kind: { type: "primary_key", columns: [], include: [] } });
    expect(newConstraint("unique", d)).toEqual({ ...b, kind: { type: "unique", columns: [], include: [], nullsNotDistinct: false } });
    expect(newConstraint("check", d)).toEqual({ ...b, kind: { type: "check", expr: "", noInherit: false } });
    expect(newConstraint("foreign_key", d)).toEqual({
      ...b, kind: { type: "foreign_key", columns: [""], refSchema: "app", refTable: "", refColumns: [""], matchType: "simple", onUpdate: "no_action", onDelete: "no_action" },
    });
  });
  it("add/update/remove: sona ekler; sınır dışı → aynı nesne; girdi değişmez", () => {
    const d = T({ constraints: [uq(["a"], 1)] });
    const e = addConstraint(d, "check");
    expect(e.constraints.map((c) => c.kind.type)).toEqual(["unique", "check"]);
    expect(d.constraints).toHaveLength(1);
    expect(updateConstraint(d, 5, (c) => c)).toBe(d);
    expect(removeConstraint(d, -1)).toBe(d);
    expect(removeConstraint(e, 0).constraints.map((c) => c.kind.type)).toEqual(["check"]);
    expect(updateConstraint(d, 0, (c) => ({ ...c, name: "x" })).constraints[0]!.name).toBe("x");
  });
  it("canAddPrimaryKey: taslakta PK varken false (yüklü ya da yeni)", () => {
    expect(canAddPrimaryKey(T())).toBe(true);
    expect(canAddPrimaryKey(T({ constraints: [pk(["a"], 1)] }))).toBe(false);
    expect(canAddPrimaryKey(T({ constraints: [pk([])] }))).toBe(false);
    expect(canAddPrimaryKey(T({ constraints: [uq(["a"])] }))).toBe(true);
  });
  it("switchConstraintType: yalnız yeni kısıtta; ad korunur; check'e geçişte deferrable=false; yüklü kısıt aynı nesne", () => {
    const d = T();
    const n = { ...uq(["a"]), name: "k", deferrable: true, initiallyDeferred: true };
    const s = switchConstraintType(n, "check", d);
    expect(s.name).toBe("k");
    expect(s.kind.type).toBe("check");
    expect(s.deferrable).toBe(false);
    expect(s.initiallyDeferred).toBe(false);
    const p = switchConstraintType(n, "primary_key", d);
    expect(p.deferrable).toBe(true);
    expect(p.initiallyDeferred).toBe(true);
    const loaded = uq(["a"], 7);
    expect(switchConstraintType(loaded, "check", d)).toBe(loaded);
    expect(switchConstraintType(n, "unique", d)).toBe(n);
    expect(switchConstraintType({ ...chk(), notValid: true }, "unique", d).notValid).toBe(false);
  });
  it("withDeferrable(false) initiallyDeferred'ı da false yapar; withInitiallyDeferred deferrable kapalıyken etkisiz", () => {
    const c = { ...pk(["a"]), deferrable: true, initiallyDeferred: true };
    expect(withDeferrable(c, false)).toMatchObject({ deferrable: false, initiallyDeferred: false });
    expect(withDeferrable(c, true).initiallyDeferred).toBe(true);
    const off = pk(["a"]);
    expect(withInitiallyDeferred(off, true)).toBe(off);
    expect(withInitiallyDeferred({ ...off, deferrable: true }, true).initiallyDeferred).toBe(true);
    expect(supportsDeferrable("check")).toBe(false);
    expect(supportsNotValid("primary_key")).toBe(false);
  });
  it("notValidMode: create+yeni → none; alter+yeni FK/CHECK → add; alter+yeni PK → none; yüklü notValid → validate; yüklü doğrulanmış → none", () => {
    expect(notValidMode("create", fk(["a"], "x", ["id"]), null)).toBe("none");
    expect(notValidMode("alter", fk(["a"], "x", ["id"]), null)).toBe("add");
    expect(notValidMode("alter", chk(), null)).toBe("add");
    expect(notValidMode("alter", pk(["a"]), null)).toBe("none");
    const nv = { ...chk(3), notValid: true };
    expect(notValidMode("alter", nv, nv)).toBe("validate");
    expect(notValidMode("alter", { ...nv, notValid: false }, nv)).toBe("validate");
    expect(notValidMode("alter", chk(3), chk(3))).toBe("none");
  });
  it("removedConstraints/restoreConstraint: özgün sırasına döner; derin kopya; defsEqual(original) → true; zaten varsa aynı nesne", () => {
    const original = T({ columns: [col("a", 1)], constraints: [pk(["a"], 1), uq(["a"], 2), chk(3)] });
    const d = { ...original, constraints: [original.constraints[0]!, chk(null)] };
    const removed = removedConstraints(original, { ...d, constraints: [chk(null)] });
    expect(removed.map((c) => c.oid)).toEqual([1, 2, 3]);
    expect(originalConstraint(original, d.constraints[0]!)).toBe(original.constraints[0]);
    expect(originalConstraint(original, chk(null))).toBeNull();
    const r = restoreConstraint(d, original, 2);
    expect(r.constraints.map((c) => c.oid)).toEqual([1, 2, null]);
    expect(r.constraints[1]).not.toBe(original.constraints[1]);
    expect(r.constraints[1]).toEqual(original.constraints[1]);
    expect(restoreConstraint(r, original, 2)).toBe(r);
    expect(restoreConstraint(d, original, 99)).toBe(d);
    let full = { ...original, constraints: [] as ConstraintDef[] };
    full = restoreConstraint(full, original, 3);
    full = restoreConstraint(full, original, 1);
    full = restoreConstraint(full, original, 2);
    expect(defsEqual({ kind: "table", ...full }, { kind: "table", ...original })).toBe(true);
    expect(removedConstraints(null, d)).toEqual([]);
  });
});

describe("kolon listeleri", () => {
  it("addConstraintColumn: boş ve yinelenen eklenmez; removeConstraintColumn sınır dışı → aynı nesne; check/fk include → []", () => {
    const c = pk(["a"]);
    expect(addConstraintColumn(c, "columns", "")).toBe(c);
    expect(addConstraintColumn(c, "columns", "a")).toBe(c);
    const e = addConstraintColumn(c, "include", "b");
    expect(constraintColumns(e, "include")).toEqual(["b"]);
    expect(constraintColumns(c, "include")).toEqual([]);
    expect(removeConstraintColumn(e, "include", 4)).toBe(e);
    expect(constraintColumns(removeConstraintColumn(e, "include", 0), "include")).toEqual([]);
    expect(constraintColumns(chk(), "columns")).toEqual([]);
    expect(constraintColumns(fk(["a"], "x", ["id"]), "include")).toEqual([]);
    const ck = chk();
    expect(addConstraintColumn(ck, "columns", "a")).toBe(ck);
  });
  it("availableColumns: boş adlar ve listede olanlar hariç; PK include seçenekleri columns'takileri içermez", () => {
    const d = T({ columns: [col("a"), col(""), col("b"), col("c")] });
    const c = uq(["a"], null, ["b"]);
    expect(availableColumns(d, c, "columns")).toEqual(["c"]);
    expect(availableColumns(d, c, "include")).toEqual(["c"]);
    expect(availableColumns(d, pk([]), "columns")).toEqual(["a", "b", "c"]);
  });
});

describe("FK", () => {
  it("FK çiftleri: add/update/remove uzunlukları eşit tutar; withFkTarget tablo değişince ref adlarını boşaltır, şema değişince de", () => {
    let c = newConstraint("foreign_key", T());
    c = addFkPair(c);
    c = updateFkPair(c, 0, { local: "a", ref: "id" });
    c = updateFkPair(c, 1, { local: "b" });
    const k = () => c.kind as Extract<typeof c.kind, { type: "foreign_key" }>;
    expect(k().columns).toEqual(["a", "b"]);
    expect(k().refColumns).toEqual(["id", ""]);
    expect(updateFkPair(c, 9, { local: "x" })).toBe(c);
    c = withFkTarget(c, { refTable: "p" });
    expect(k().refColumns).toEqual(["", ""]);
    c = updateFkPair(c, 0, { ref: "id" });
    expect(withFkTarget(c, { refTable: "p" })).toBe(c);
    expect(k().refColumns).toEqual(["id", ""]);
    c = withFkTarget(c, { refSchema: "s" });
    expect(k().refColumns).toEqual(["", ""]);
    expect(k().columns).toHaveLength(2);
    c = removeFkPair(c, 0);
    expect(k().columns).toEqual(["b"]);
    expect(k().refColumns).toHaveLength(1);
    expect(removeFkPair(c, 5)).toBe(c);
    const a = withFkActions(c, { onDelete: "cascade", matchType: "full" });
    expect(a.kind).toMatchObject({ onDelete: "cascade", matchType: "full", onUpdate: "no_action" });
    const u = uq(["a"]);
    expect(addFkPair(u)).toBe(u);
    expect(withFkActions(u, { onDelete: "cascade" })).toBe(u);
  });
  it("danglingColumns: silinen kolona başvuran PK/UNIQUE/FK; kendine-başvuran FK refColumns da; başka tabloya FK refColumns hariç; boş adlar hariç", () => {
    const d = T({ columns: [col("a")] });
    expect(danglingColumns(d, pk(["a", "z"]))).toEqual(["z"]);
    expect(danglingColumns(d, uq(["a", ""], null, ["y", "y"]))).toEqual(["y"]);
    expect(danglingColumns(d, fk(["a", "q"], "other", ["nope", ""]))).toEqual(["q"]);
    expect(danglingColumns(d, fk(["a"], "t", ["nope"]))).toEqual(["nope"]);
    expect(danglingColumns(d, fk(["a"], "t", ["nope"]), false)).toEqual([]);
    expect(danglingColumns(d, fk(["a"], "other", ["nope"]), true)).toEqual(["nope"]);
    expect(danglingColumns(d, chk())).toEqual([]);
  });
  it("konum anlık görüntüsü: geçici tablo adı başka tablonun FK'sini yakalamaz (selfRefs + renameTable + danglingColumns(selfRef))", () => {
    const d = T({ name: "orders", columns: [col("a")], constraints: [fk(["a"], "orders", ["a"]), fk(["a"], "customers", ["id"])] });
    const snap = selfRefs(d); // düzenleme başlarken
    expect(snap).toEqual([0]);
    const mid = renameTable(d, { name: "customers" }, snap); // yazım sırasında ara ad
    expect(mid.constraints[1]!.kind).toMatchObject({ refTable: "customers" });
    const fin = renameTable(mid, { name: "orders2" }, snap);
    expect(fin.constraints[0]!.kind).toMatchObject({ refTable: "orders2" });
    expect(fin.constraints[1]!.kind).toMatchObject({ refTable: "customers" });
    // ara adda ad-bazlı sezgi başka tablonun FK'sini kendine sayardı; anlık görüntü sayması
    expect(danglingColumns(mid, mid.constraints[1]!, snap.includes(1))).toEqual([]);
    expect(danglingColumns(mid, mid.constraints[1]!)).toEqual(["id"]);
  });
});

describe("PRIMARY KEY notNull yardımcısı", () => {
  it("PK üyesi kolonları notNull=true yapar; diğerlerine ve değişmeyene dokunmaz", () => {
    const d = T({ columns: [col("a", 1), col("b", 2), col("c", 3, true)], constraints: [pk(["a", "c"]), uq(["b"])] });
    const e = withPrimaryKeyNotNull(d);
    expect(e.columns.map((c) => c.notNull)).toEqual([true, false, true]);
    expect(e.columns[2]).toBe(d.columns[2]);
    expect(d.columns[0]!.notNull).toBe(false);
    expect(primaryKeyNullable(d)).toEqual(["a"]);
    expect(withPrimaryKeyNotNull(e)).toBe(e);
    expect(primaryKeyNullable(e)).toEqual([]);
    const none = T({ columns: [col("a")] });
    expect(withPrimaryKeyNotNull(none)).toBe(none);
    const blank = T({ columns: [col("")], constraints: [pk([""])] });
    expect(withPrimaryKeyNotNull(blank)).toBe(blank);
  });
});

describe("görünüm yardımcıları", () => {
  it("unsupportedKind: yükleyici etiketleri", () => {
    expect(unsupportedKind("NOT NULL c (NOT VALID)")).toBe("notNull");
    expect(unsupportedKind("EXCLUDE ex")).toBe("exclude");
    expect(unsupportedKind("CONSTRAINT TRIGGER t")).toBe("trigger");
    expect(unsupportedKind("kimlik dizisi seçenekleri (id)")).toBe("identity");
    expect(unsupportedKind("WITH (autovacuum_enabled=false)")).toBe("storage");
    expect(unsupportedKind("OF mytype (tipli tablo)")).toBe("typed");
    expect(unsupportedKind("CHECK c (NOT ENFORCED)")).toBe("other");
    expect(unsupportedKind("'z' türü kısıt q")).toBe("other");
  });
  it("constraintLabel: ad varsa ad; yoksa 'adsız PRIMARY KEY' vb.", () => {
    expect(constraintLabel({ ...pk(["a"]), name: "pk_t" })).toBe("pk_t");
    expect(constraintLabel(pk(["a"]))).toBe("adsız PRIMARY KEY");
    expect(constraintLabel(fk([""], "", [""]))).toBe("adsız FOREIGN KEY");
    expect(constraintLabel({ ...chk(), name: "" })).toBe("adsız CHECK");
  });
});

describe("tur-guvenli duzeltmeler (D2-D7)", () => {
  it("snapshot: ara ad + kisit sil/geri al + yeni self FK; son ad yalniz gercek self FK'lere yazilir", () => {
    const original = T({ name: "orders", columns: [col("a", 1)], constraints: [chk(1), fk(["a"], "orders", ["a"], 2), fk(["a"], "customers", ["id"], 3)] });
    let d = original;
    let snap: SelfSnap = selfRefs(d);
    expect(snap).toEqual([1]);
    d = renameTable(d, { name: "customers" }, snap);
    ({ d, snap } = removeConstraintSnap(d, 0, snap));
    expect(snap).toEqual([0]);
    ({ d, snap } = restoreConstraintSnap(d, original, 1, snap));
    expect(d.constraints.map((c) => c.oid)).toEqual([1, 2, 3]);
    expect(snap).toEqual([1]);
    ({ d, snap } = addConstraintSnap(d, "foreign_key", snap));
    ({ d, snap } = pointFkAtSelf(d, 3, snap));
    expect(snap).toEqual([1, 3]);
    d = renameTable(d, { name: "orders2" }, snap);
    const refs = d.constraints.map((c) => (c.kind.type === "foreign_key" ? c.kind.refTable : null));
    expect(refs).toEqual([null, "orders2", "customers", "orders2"]);
    ({ d, snap } = pointFkAtOther(d, 3, { refTable: "x" }, snap));
    expect(snap).toEqual([1]);
    // geri alinan self FK adi guncel taslak adina hizalanir
    ({ d, snap } = removeConstraintSnap(d, 1, snap));
    expect(snap).toEqual([]);
    ({ d, snap } = restoreConstraintSnap(d, original, 2, snap));
    expect(snap).toEqual([1]);
    expect(d.constraints[1]!.kind).toMatchObject({ refTable: "orders2" });
    expect(removeConstraintSnap(d, 99, snap).d).toBe(d);
  });
  it("canRestoreConstraint: ikinci PK engellenir", () => {
    const original = T({ constraints: [pk(["a"], 1), uq(["a"], 2)] });
    const withOther = T({ constraints: [pk(["b"])] });
    expect(canRestoreConstraint(withOther, original, 1)).toBe(false);
    expect(canRestoreConstraint(withOther, original, 2)).toBe(true);
    expect(canRestoreConstraint(T(), original, 1)).toBe(true);
    expect(canRestoreConstraint(original, original, 1)).toBe(false);
    expect(canRestoreConstraint(T(), original, 9)).toBe(false);
  });
  it("ayni nesne: no-op'lar", () => {
    const f = fk(["a"], "x", ["id"]);
    expect(withFkActions(f, { onDelete: "no_action" })).toBe(f);
    expect(updateFkPair(f, 0, { local: "a", ref: "id" })).toBe(f);
    const d = T({ constraints: [f] });
    expect(updateConstraint(d, 0, (c) => c)).toBe(d);
    const p = { ...pk(["a"]), deferrable: true };
    expect(withDeferrable(p, true)).toBe(p);
    expect(withInitiallyDeferred(p, false)).toBe(p);
  });
  it("withDeferrable CHECK'te etkisiz", () => {
    const c = chk();
    expect(withDeferrable(c, true)).toBe(c);
  });
  it("canAddPrimaryKey(excludingIndex) ve switch -> PK ikinci PK'yi reddeder", () => {
    const d = T({ constraints: [pk(["a"]), uq(["b"])] });
    expect(canAddPrimaryKey(d, 0)).toBe(true);
    expect(canAddPrimaryKey(d, 1)).toBe(false);
    expect(switchConstraintType(d.constraints[1]!, "primary_key", d)).toBe(d.constraints[1]);
    const solo = T({ constraints: [uq(["b"])] });
    expect(switchConstraintType(solo.constraints[0]!, "primary_key", solo).kind.type).toBe("primary_key");
  });
});
