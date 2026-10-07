import { describe, expect, it } from "vitest";
import { ttbl } from "../../app/strings-table";
import type { ColumnDef } from "../../ipc/bindings/ColumnDef";
import type { ConstraintDef } from "../../ipc/bindings/ConstraintDef";
import type { TableDef } from "../../ipc/bindings/TableDef";
import { defsEqual, groupIssues } from "./designer-model";
import {
  addPartitionKey, addTableColumn, partitionColumnChoices, partitionKeyLabel, allowedValueSources, asTable, columnRefs, newColumn, originalColumn, removePartitionKey, removeTableColumn,
  removedColumns, renameTable, renameTableColumn, restoreColumn, rowOfFocus, sectionHasIssues, selfRefs, serialFix, storageGate, structureLocked,
  switchPartitionKeyKind, tableEditPatch, tableSections, typeQueryOf, typeTextWarning, updatePartitionKey, updateTableColumn, usingVisible,
  valueExpr, valueSourceOf, withCollation, withDataType, withPartitionStrategy, withValueExpr, withValueSource,
} from "./table-model";

const T = (p: Partial<TableDef> = {}): TableDef => ({
  schema: "public", name: "t", owner: null, tablespace: null, persistence: "permanent", columns: [], constraints: [], partitionBy: null,
  fillfactor: null, comment: null, partitionText: null, isPartition: false, inherits: false, unsupported: [], ...p,
});
const col = (name: string, attnum: number | null, p: Partial<ColumnDef> = {}): ColumnDef => ({ ...newColumn(), name, attnum, dataType: "integer", ...p });
const uq = (columns: string[], oid: number | null, include: string[] = []): ConstraintDef => ({
  oid, name: null, kind: { type: "unique", columns, include, nullsNotDistinct: false }, deferrable: false, initiallyDeferred: false, notValid: false,
});
const pk = (columns: string[], oid: number | null): ConstraintDef => ({
  oid, name: null, kind: { type: "primary_key", columns, include: [] }, deferrable: false, initiallyDeferred: false, notValid: false,
});
const fk = (columns: string[], refSchema: string, refTable: string, refColumns: string[], oid: number | null): ConstraintDef => ({
  oid, name: null, kind: { type: "foreign_key", columns, refSchema, refTable, refColumns, matchType: "simple", onUpdate: "no_action", onDelete: "no_action" },
  deferrable: false, initiallyDeferred: false, notValid: false,
});
const chk = (expr: string, oid: number | null): ConstraintDef => ({
  oid, name: null, kind: { type: "check", expr, noInherit: false }, deferrable: false, initiallyDeferred: false, notValid: false,
});

describe("kolon işlemleri", () => {
  it("addTableColumn sona ekler; newColumn attnum=null ve tüm isteğe bağlılar null", () => {
    const d = addTableColumn(T({ columns: [col("a", 1)] }));
    expect(d.columns.map((c) => c.name)).toEqual(["a", ""]);
    const n = newColumn();
    expect(n).toEqual({ attnum: null, name: "", dataType: "", notNull: false, default: null, identity: null, generated: null, collation: null, comment: null, using: null });
  });
  it("removeTableColumn/updateTableColumn değişmez; sınır dışı indeks aynı nesneyi döner", () => {
    const d = T({ columns: [col("a", 1), col("b", 2)] });
    const u = updateTableColumn(d, 1, (c) => ({ ...c, name: "x" }));
    expect(u.columns[1]!.name).toBe("x");
    expect(d.columns[1]!.name).toBe("b");
    expect(removeTableColumn(d, 0).columns.map((c) => c.name)).toEqual(["b"]);
    expect(d.columns).toHaveLength(2);
    for (const i of [-1, 2, 1.5, NaN]) {
      expect(removeTableColumn(d, i)).toBe(d);
      expect(updateTableColumn(d, i, (c) => c)).toBe(d);
    }
  });
  it("removedColumns: taslakta olmayan yüklü kolonlar özgün sırayla; yeni kolonlar sayılmaz", () => {
    const o = T({ columns: [col("a", 1), col("b", 2), col("c", 3)] });
    const d = addTableColumn(removeTableColumn(removeTableColumn(o, 1), 0));
    expect(removedColumns(o, d).map((c) => c.name)).toEqual(["a", "b"]);
    expect(removedColumns(null, d)).toEqual([]);
    expect(originalColumn(o, d.columns[0]!)?.name).toBe("c");
    expect(originalColumn(o, d.columns[1]!)).toBeNull();
  });
  it("restoreColumn: attnum sırasındaki yerine (yeni kolonlardan önce) özgün nesnenin derin kopyasıyla döner; defsEqual(original) → true", () => {
    const o = T({ columns: [col("a", 1), col("b", 2), col("c", 3)] });
    let d = removeTableColumn(removeTableColumn(o, 1), 0); // [c]
    d = addTableColumn(d); // [c, yeni]
    d = restoreColumn(restoreColumn(d, o, 2), o, 1); // [a, b, c, yeni]
    expect(d.columns.map((c) => c.name)).toEqual(["a", "b", "c", ""]);
    expect(d.columns[0]).not.toBe(o.columns[0]);
    expect(restoreColumn(d, o, 2)).toBe(d); // zaten var
    expect(restoreColumn(d, o, 99)).toBe(d); // bulunamaz
    expect(defsEqual({ kind: "table", ...removeTableColumn(d, 3) }, { kind: "table", ...o })).toBe(true);
  });
});

describe("değer kaynağı", () => {
  it("valueSourceOf/withValueSource: ifade metni default↔generated taşınır; identity notNull=true yapar; none hepsini temizler", () => {
    const c = col("a", null, { default: "now()" });
    expect(valueSourceOf(c)).toBe("default");
    const g = withValueSource(c, "generated_stored");
    expect(g.generated).toEqual({ expr: "now()", stored: true });
    expect(g.default).toBeNull();
    expect(valueSourceOf(g)).toBe("generated_stored");
    expect(valueSourceOf(withValueSource(g, "generated_virtual"))).toBe("generated_virtual");
    expect(valueExpr(withValueSource(g, "default"))).toBe("now()");
    const i = withValueSource(c, "identity_always");
    expect(i).toMatchObject({ identity: "always", notNull: true, default: null, generated: null });
    expect(withValueSource(c, "identity_by_default").identity).toBe("by_default");
    expect(withValueSource(i, "none")).toMatchObject({ identity: null, default: null, generated: null });
    expect(valueSourceOf(newColumn())).toBe("none");
    expect(valueExpr(newColumn())).toBe("");
  });
  it("withValueExpr kaynağa göre yazar; boş metin '' kalır", () => {
    expect(withValueExpr(col("a", null, { default: "1" }), "").default).toBe("");
    expect(withValueExpr(col("a", null, { generated: { expr: "x", stored: true } }), "y").generated).toEqual({ expr: "y", stored: true });
    const n = col("a", null);
    expect(withValueExpr(n, "z")).toBe(n);
  });
  it("allowedValueSources: yeni → 6 seçenek; yüklü düz → none/default/identity_*; yüklü stored → none + generated_stored", () => {
    expect(allowedValueSources(null)).toHaveLength(6);
    expect(allowedValueSources(col("a", 1))).toEqual(["none", "default", "identity_always", "identity_by_default"]);
    expect(allowedValueSources(col("a", 1, { generated: { expr: "1", stored: true } }))).toEqual(["none", "generated_stored"]);
    expect(allowedValueSources(col("a", 1, { generated: { expr: "1", stored: false } }))).toEqual(["none", "generated_virtual"]);
  });
});

describe("tip", () => {
  it("withDataType/withCollation: tip özgüne dönünce using null; farklıyken using korunur; usingVisible yalnız yüklü+değişmiş", () => {
    const o = col("a", 1, { dataType: "integer" });
    let c = withDataType({ ...o, using: "a::bigint" }, o, "bigint");
    expect(c.using).toBe("a::bigint");
    expect(usingVisible(o, c)).toBe(true);
    c = withDataType(c, o, "integer");
    expect(c.using).toBeNull();
    expect(usingVisible(o, c)).toBe(false);
    const k = withCollation({ ...o, using: "x" }, o, "C");
    expect(k.collation).toBe("C");
    expect(k.using).toBe("x");
    expect(withCollation(k, o, "  ")).toMatchObject({ collation: null, using: null });
    expect(usingVisible(null, col("n", null, { dataType: "text" }))).toBe(false);
    expect(withDataType({ ...col("n", null), using: "q" }, null, "text").using).toBe("q");
  });
  it("typeTextWarning kabul", () => {
    for (const t of [
      "integer", "double precision", "character varying(50)", "numeric(10,2)", "numeric(10,-2)", "timestamp(3) with time zone",
      "public.mytype", "\"My Type\"[]", "int4[][]", "numeric(10,2)[3]",
    ]) expect(typeTextWarning(t), t).toBeNull();
  });
  it("typeTextWarning ret (doğru metinle)", () => {
    const w = ttbl.type;
    const cases: Array<[string, string]> = [
      ["", w.required], ["x".repeat(129), w.tooLong], ["int;", w.forbidden(";")], ["int'x'", w.forbidden("'")], ["text --", w.comment],
      ["a/*b*/", w.comment], ["numeric(1;2)", w.forbidden(";")], ["numeric(a)", w.modifier], ["int[", w.array], ["\"unterminated", w.unclosedQuote],
      ["a\nb", w.control], ["int  4", w.spaces], [" int", w.spaces], ["x[1]y", w.array], ["numeric(1)(2)", w.modifier], ["serial", w.serial],
      ["BIGSERIAL", w.serial], ["pg_catalog.serial4", w.serial],
    ];
    for (const [t, msg] of cases) expect(typeTextWarning(t), JSON.stringify(t)).toBe(msg);
  });
  it("serialFix: serial/serial4→integer, bigserial/serial8→bigint, smallserial/serial2→smallint; 'serial[]' ve 'myserial' null", () => {
    expect(serialFix("serial")).toEqual({ dataType: "integer" });
    expect(serialFix("serial4")).toEqual({ dataType: "integer" });
    expect(serialFix("BigSerial")).toEqual({ dataType: "bigint" });
    expect(serialFix("serial8")).toEqual({ dataType: "bigint" });
    expect(serialFix("smallserial")).toEqual({ dataType: "smallint" });
    expect(serialFix("pg_catalog.serial2")).toEqual({ dataType: "smallint" });
    expect(serialFix("serial[]")).toBeNull();
    expect(serialFix("myserial")).toBeNull();
  });
  it("typeQueryOf: ilk ( / [ öncesi; boş → null", () => {
    expect(typeQueryOf(" numeric(10,2) ")).toBe("numeric");
    expect(typeQueryOf("int4[]")).toBe("int4");
    expect(typeQueryOf("")).toBeNull();
    expect(typeQueryOf("(x")).toBeNull();
    expect(typeQueryOf("double precision")).toBe("double precision");
  });
});

describe("yeniden adlandırma yayılımı", () => {
  it("columnRefs: PK/UNIQUE columns+include, FK columns; kendine-başvuran FK refColumns; başka tabloya FK refColumns hariç; CHECK hariç; bölümleme kolon anahtarı", () => {
    const d = T({
      columns: [col("a", 1)],
      constraints: [pk(["a"], 1), uq(["x"], 2, ["a"]), fk(["a"], "public", "t", ["a"], 3), fk(["a"], "public", "other", ["a"], 4), chk("a > 0", 5)],
      partitionBy: { strategy: "range", keys: [{ kind: "column", name: "a" }, { kind: "expression", sql: "a" }] },
    });
    expect(columnRefs(d, "a", selfRefs(d))).toEqual([
      { at: "constraint", c: 0, list: "columns", j: 0 },
      { at: "constraint", c: 1, list: "include", j: 0 },
      { at: "constraint", c: 2, list: "columns", j: 0 },
      { at: "constraint", c: 2, list: "refColumns", j: 0 },
      { at: "constraint", c: 3, list: "columns", j: 0 },
      { at: "partitionKey", j: 0 },
    ]);
    expect(columnRefs(d, "", selfRefs(d))).toEqual([]);
    // Ad hedefle eşleşse de anlık görüntüde olmayan FK'nin refColumns'u dahil edilmez.
    expect(columnRefs(d, "a", []).some((r) => r.at === "constraint" && r.list === "refColumns")).toBe(false);
  });
  it("renameTableColumn: konum anlık görüntüsü — ara ad başka kolonun adıyla çakışsa da yalnızca anlık görüntüdeki konumlar güncellenir", () => {
    const d0 = T({ columns: [col("a", 1), col("ab", 2)], constraints: [uq(["a"], 10), uq(["ab"], 11)] });
    const refs = columnRefs(d0, "a", []);
    const d1 = renameTableColumn(d0, 0, "ab", refs);
    const d2 = renameTableColumn(d1, 0, "abc", refs);
    expect(d2.constraints.map((k) => (k.kind as { columns: string[] }).columns)).toEqual([["abc"], ["ab"]]);
    expect(d0.constraints.map((k) => (k.kind as { columns: string[] }).columns)).toEqual([["a"], ["ab"]]);
    expect(d2.columns[0]!.name).toBe("abc");
  });
  it("renameTableColumn: bölümleme anahtarı ve sınır dışı indeks", () => {
    const d = T({ columns: [col("a", 1)], partitionBy: { strategy: "list", keys: [{ kind: "column", name: "a" }] } });
    const r = renameTableColumn(d, 0, "z", columnRefs(d, "a", []));
    expect(r.partitionBy!.keys[0]).toEqual({ kind: "column", name: "z" });
    expect(renameTableColumn(d, 5, "z", [])).toBe(d);
  });
  it("selfRefs/renameTable: tabloyu hedefleyen FK'lerin refSchema/refTable'ı adla birlikte güncellenir; başka tabloya FK dokunulmaz", () => {
    const d = T({ constraints: [fk(["a"], "public", "t", ["id"], 1), fk(["b"], "public", "other", ["id"], 2), uq(["a"], 3)] });
    expect(selfRefs(d)).toEqual([0]);
    const r = renameTable(d, { name: "t2", schema: "s2" }, selfRefs(d));
    expect(r).toMatchObject({ name: "t2", schema: "s2" });
    expect(r.constraints[0]!.kind).toMatchObject({ refSchema: "s2", refTable: "t2" });
    expect(r.constraints[1]).toBe(d.constraints[1]);
    expect(d.constraints[0]!.kind).toMatchObject({ refTable: "t" });
    const only = renameTable(d, { name: "t3" }, []);
    expect(only.constraints[0]!.kind).toMatchObject({ refTable: "t" });
  });
});

describe("bölümleme", () => {
  it("null/yok→var/strateji değişimi anahtarları korur; anahtar ekle/sil/güncelle/tür değiştir (metin taşınır)", () => {
    const d = T();
    expect(withPartitionStrategy(d, null)).toMatchObject({ partitionBy: null });
    let p = withPartitionStrategy(d, "range");
    expect(p.partitionBy).toEqual({ strategy: "range", keys: [] });
    p = addPartitionKey(p, "column");
    p = addPartitionKey(p, "expression");
    expect(p.partitionBy!.keys).toEqual([{ kind: "column", name: "" }, { kind: "expression", sql: "" }]);
    p = updatePartitionKey(p, 0, { kind: "column", name: "a" });
    p = withPartitionStrategy(p, "hash");
    expect(p.partitionBy).toMatchObject({ strategy: "hash" });
    expect(p.partitionBy!.keys).toHaveLength(2);
    expect(switchPartitionKeyKind({ kind: "column", name: "a" }, "expression")).toEqual({ kind: "expression", sql: "a" });
    expect(switchPartitionKeyKind({ kind: "expression", sql: "f(a)" }, "column")).toEqual({ kind: "column", name: "f(a)" });
    const k = { kind: "column", name: "a" } as const;
    expect(switchPartitionKeyKind(k, "column")).toBe(k);
    p = removePartitionKey(p, 0);
    expect(p.partitionBy!.keys).toEqual([{ kind: "expression", sql: "" }]);
    expect(removePartitionKey(p, 9)).toBe(p);
    expect(updatePartitionKey(p, 9, k)).toBe(p);
    expect(addPartitionKey(d, "column")).toBe(d);
  });
});

describe("odak / bölümler / genel", () => {
  it("rowOfFocus: column:2 → columns bölümü ve attnum=2 satırı; constraint:40 → oid; bulunamazsa index null; bozuk alan → null", () => {
    const d = T({ columns: [col("a", 1), col("b", 2)], constraints: [pk(["a"], 30), uq(["b"], 40)] });
    expect(rowOfFocus(d, "column:2")).toEqual({ section: "columns", index: 1 });
    expect(rowOfFocus(d, "constraint:40")).toEqual({ section: "constraints", index: 1 });
    expect(rowOfFocus(d, "column:9")).toEqual({ section: "columns", index: null });
    expect(rowOfFocus(d, "constraint:9")).toEqual({ section: "constraints", index: null });
    expect(rowOfFocus(d, "name")).toBeNull();
  });
  it("tableSections: create → 4 bölüm; alter → ilk üçü", () => {
    expect(tableSections("create")).toEqual(["general", "columns", "constraints", "partitioning"]);
    expect(tableSections("alter")).toEqual(["general", "columns", "constraints"]);
  });
  it("sectionHasIssues", () => {
    const g = (paths: string[]) => groupIssues(paths.map((path) => ({ path, message: "m" })));
    expect(sectionHasIssues(g(["columns[1].dataType"]), "columns")).toBe(true);
    expect(sectionHasIssues(g(["columns[1].dataType"]), "constraints")).toBe(false);
    expect(sectionHasIssues(g(["constraints[0].expr"]), "constraints")).toBe(true);
    expect(sectionHasIssues(g(["partitionBy.keys[0]"]), "partitioning")).toBe(true);
    expect(sectionHasIssues(g(["name"]), "general")).toBe(true);
    expect(sectionHasIssues(g(["name"]), "columns")).toBe(false);
    expect(sectionHasIssues(g([]), "general")).toBe(false);
    // Üst-düzey `constraints` (ör. bölüm/kalıtımlı tablo kısıt farkı) Kısıtlar sekmesini işaretler, Genel'i değil.
    const top = groupIssues([{ path: "constraints", message: "m" }], new Set(["name", "constraints"]));
    expect([sectionHasIssues(top, "constraints"), sectionHasIssues(top, "general")]).toEqual([true, false]);
  });
  it("structureLocked: isPartition ya da inherits → true; create (original null) → false", () => {
    expect(structureLocked(T({ isPartition: true }))).toBe(true);
    expect(structureLocked(T({ inherits: true }))).toBe(true);
    expect(structureLocked(T())).toBe(false);
    expect(structureLocked(null)).toBe(false);
  });
  it("asTable/tableEditPatch tür uyuşmazlığında değişmez", () => {
    const t = { kind: "table" as const, ...T() };
    expect(asTable(t)).toBe(t);
    expect(asTable(null)).toBeNull();
    expect(tableEditPatch(t, { name: "x" })).toMatchObject({ kind: "table", name: "x" });
    const s = { kind: "schema" as const, name: "s", owner: null, comment: null };
    expect(asTable(s)).toBeNull();
    expect(tableEditPatch(s, { name: "x" })).toBe(s);
  });
});

describe("düzeltme turu", () => {
  it("withValueSource(original): yüklü kaynağa dönüş birebir geri koyar; kimliğin zorladığı notNull geri alınır", () => {
    const o = col("a", 1, { notNull: false });
    const viaIdentity = withValueSource(withValueSource(o, "identity_always", o), "none", o);
    expect(viaIdentity).toEqual(o);
    const od = col("a", 1, { default: "now()" });
    const back = withValueSource(withValueSource(od, "none", od), "default", od);
    expect(back.default).toBe("now()");
    expect(back).toEqual(od);
    const og = col("a", 1, { generated: { expr: "b * 2", stored: true } });
    const g = withValueSource(withValueSource(og, "none", og), "generated_stored", og);
    expect(g).toEqual(og);
    expect(g.generated).not.toBe(og.generated);
    expect(defsEqual({ kind: "table", ...T({ columns: [viaIdentity] }) }, { kind: "table", ...T({ columns: [o] }) })).toBe(true);
    // kullanıcının kendi notNull seçimi (kimlik dışı) korunur
    expect(withValueSource({ ...o, notNull: true }, "default", o).notNull).toBe(true);
  });
  it("withValueSource/renameTable girdiyi değiştirmez", () => {
    const o = col("a", 1, { default: "1" });
    const snap = structuredClone(o);
    withValueSource(o, "identity_always", o); withValueSource(o, "generated_stored", o); withValueSource(o, "none");
    expect(o).toEqual(snap);
    const d = T({ constraints: [fk(["a"], "public", "t", ["id"], 1)] });
    const ds = structuredClone(d);
    renameTable(d, { name: "x", schema: "y" }, [0]);
    expect(d).toEqual(ds);
  });
  it("typeTextWarning: tırnaklı/parantez içi çift boşluk uyarmaz; uzunluk karakter sayar", () => {
    expect(typeTextWarning("\"My  Type\"")).toBeNull();
    expect(typeTextWarning("numeric(10,  2)")).toBeNull();
    expect(typeTextWarning("int  4")).toBe(ttbl.type.spaces);
    expect(typeTextWarning("\"" + "😀".repeat(100) + "\"")).toBeNull();
    expect(typeTextWarning("x".repeat(128))).not.toBe(ttbl.type.tooLong);
  });
  it("usingVisible/using sıfırlama: biçim farkları (INTEGER, boşluk) tip değişimi sayılmaz", () => {
    const o = col("a", 1, { dataType: "numeric(10,2)" });
    expect(usingVisible(o, { ...o, dataType: "NUMERIC(10, 2)" })).toBe(false);
    expect(usingVisible(o, { ...o, dataType: "  numeric(10,2) " })).toBe(false);
    expect(usingVisible(o, { ...o, dataType: "bigint" })).toBe(true);
    expect(withDataType({ ...o, using: "x" }, o, "NUMERIC(10,2)").using).toBeNull();
    const q = col("a", 1, { dataType: "\"My Type\"" });
    expect(usingVisible(q, { ...q, dataType: "\"my type\"" })).toBe(true);
  });
});

describe("bölümleme anahtarı etiketi ve kolon seçenekleri", () => {
  it("etiket: ad/ifade, boşluk sadeleşir, boş → 'boş', uzun kısaltılır", () => {
    expect(partitionKeyLabel({ kind: "column", name: "created_at" })).toBe("created_at");
    expect(partitionKeyLabel({ kind: "expression", sql: "  date_trunc('month',\n  x) " })).toBe("date_trunc('month', x)");
    expect(partitionKeyLabel({ kind: "column", name: "" })).toBe(ttbl.partitioning.emptyKey);
    expect(partitionKeyLabel({ kind: "expression", sql: "a".repeat(50) })).toBe(`${"a".repeat(39)}…`);
  });
  it("seçenekler: boş ve yinelenen adlar elenir; seçili değer listede yoksa başa eklenir", () => {
    const d = T({ columns: [col("a", null), col("", null), col("b", null), col("a", null)] });
    expect(partitionColumnChoices(d, "a")).toEqual(["a", "b"]);
    expect(partitionColumnChoices(d, "")).toEqual(["", "a", "b"]);
    expect(partitionColumnChoices(d, "gone")).toEqual(["gone", "a", "b"]);
  });
});

describe("C-1: kolon yeniden adlandırma kendine-başvuruyu adla değil anlık görüntüyle belirler", () => {
  it("tablo t→other yapılınca public.other'a giden FK'nin refColumns'u kolon yeniden adlandırmada değişmez", () => {
    const d0 = T({ columns: [col("id", 1)], constraints: [fk(["id"], "public", "t", ["id"], 1), fk(["id"], "public", "other", ["id"], 2)] });
    const snap = selfRefs(d0); // [0]
    const d1 = renameTable(d0, { name: "other" }, snap);
    const d2 = renameTableColumn(d1, 0, "id2", columnRefs(d1, "id", snap));
    expect((d2.constraints[0]!.kind as { refColumns: string[] }).refColumns).toEqual(["id2"]);
    expect((d2.constraints[1]!.kind as { refColumns: string[] }).refColumns).toEqual(["id"]);
    expect((d2.constraints[1]!.kind as { columns: string[] }).columns).toEqual(["id2"]);
  });
});

describe("storageGate (bölümlenmiş tablo depolama kuralları)", () => {
  it("bölümlenmemiş: kilit/ipucu yok", () => {
    expect(storageGate("create", null, T())).toEqual({
      persistenceLocked: false, fillfactorLocked: false, pgDefaultBlocked: false, persistenceHint: null, fillfactorHint: null, tablespaceHint: null,
    });
    expect(storageGate("alter", T(), T()).persistenceLocked).toBe(false);
  });
  it("alter, yüklenen bölümlenmiş tablo: kalıcılık/fillfactor yüklenen değerdeyken kilitli; değişmişse açık (geri alınabilsin)", () => {
    const o = T({ partitionText: "RANGE (a)" });
    const g = storageGate("alter", o, o);
    expect([g.persistenceLocked, g.fillfactorLocked, g.pgDefaultBlocked]).toEqual([true, true, false]);
    expect([g.persistenceHint, g.fillfactorHint, g.tablespaceHint]).toEqual([
      ttbl.general.partitionedPersistence, ttbl.general.partitionedFillfactor, ttbl.general.partitionedTablespaceAlter,
    ]);
    const changed = storageGate("alter", o, { ...o, persistence: "unlogged", fillfactor: 70 });
    expect([changed.persistenceLocked, changed.fillfactorLocked]).toEqual([false, false]);
  });
  it("create + partitionBy: fillfactor boşken kilitli, pg_default kapalı, UNLOGGED yalnızca ipucu (sürüme bağlı)", () => {
    const d = T({ partitionBy: { strategy: "range", keys: [] } });
    const g = storageGate("create", null, d);
    expect([g.persistenceLocked, g.fillfactorLocked, g.pgDefaultBlocked]).toEqual([false, true, true]);
    expect(g.persistenceHint).toBe(ttbl.general.partitionedUnlogged);
    expect(storageGate("create", null, { ...d, fillfactor: 80 }).fillfactorLocked).toBe(false);
  });
});
