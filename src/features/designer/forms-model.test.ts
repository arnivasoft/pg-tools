import { describe, expect, it } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { ExtensionDef } from "../../ipc/bindings/ExtensionDef";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { SchemaDef } from "../../ipc/bindings/SchemaDef";
import type { SequenceDef } from "../../ipc/bindings/SequenceDef";
import { addColumn, groupIssues, issuesFor, moveColumn, removeColumn, updateColumn } from "./designer-model";
import {
  DATABASE_PATHS, EXTENSION_PATHS, FORM_PATHS, ROLE_PATHS, ROUTINE_PATHS, TRIGGER_PATHS, TYPE_PATHS, INDEX_PATHS, SCHEMA_PATHS, SEQUENCE_PATHS, TABLE_PATHS, VIEW_PATHS, MATVIEW_PATHS, asExtension, asIndex, asSchema, asSequence, btreeOnlyFields,
  columnKeyLabel, exactIssues, extensionEditPatch, extensionOptionLabel, extensionVersionNote, fillfactorEnabled,
  fillfactorFromInput, findExtension, incrementEditPatch, incrementSign, indexEditPatch, indexMethodPatch, indexReadOnlyReason,
  newIndexColumn, normalizeEmpty, normalizeTablespaceEdit, nullsFirstToSelect, orphanRowIssues, ownedByHint, restrictedControlDisabled, schemaEditPatch, sequenceSchemaPatch, tablespaceChoices,
  selectToNullsFirst, seqTypeBoundsNote, sequenceDirectionChanged, sequenceEditPatch, sequenceFormatWarnings,
  sequenceStartChanged, sequenceTypeChanged, structuralChange, switchKeyKind, updateInclude,
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

describe("sahip girdisi (normalizeEmpty)", () => {
  it("yazım sırasında sondaki boşluk korunur, yalnız boşluk → null", () => {
    expect(normalizeEmpty("my ")).toBe("my ");
    expect(normalizeEmpty("my role")).toBe("my role");
    expect(normalizeEmpty("   ")).toBeNull();
  });
});

// ---- Task 6: Sequence ve Index ----

const seq = (p: Partial<SequenceDef> = {}): SequenceDef => ({
  schema: "app", name: "s", owner: null, dataType: "bigint", increment: "1", minValue: null, maxValue: null, start: "1",
  cache: "1", cycle: false, ownedBy: null, comment: null, ...p,
});
const seqObj = (p: Partial<SequenceDef> = {}): ObjectDef => ({ kind: "sequence", ...seq(p) });
const col = (name: string, p: Partial<IndexColumn> = {}): IndexColumn => ({
  key: { kind: "column", name }, opclass: null, collation: null, descending: false, nullsFirst: null, ...p,
});
const idx = (p: Partial<IndexDef> = {}): IndexDef => ({
  schema: "app", table: "t", name: "i", method: "btree", unique: false, columns: [col("a")], include: [], whereClause: null,
  tablespace: null, fillfactor: null, comment: null, constraint: null, valid: true, ...p,
});

describe("incrementEditPatch (yön değişimi)", () => {
  it("artan → azalan: boş/varsayılan start/min/max azalan varsayılanına çekilir", () => {
    const n = incrementEditPatch(seq(), "-1");
    expect(n).toMatchObject({ increment: "-1", start: "-1", minValue: null, maxValue: "-1" });
    expect(incrementEditPatch(seq({ minValue: "1", start: "" }), "-5")).toMatchObject({ minValue: null, start: "-1", maxValue: "-1" });
  });
  it("elle girilmiş değerler korunur", () => {
    const n = incrementEditPatch(seq({ start: "100", minValue: "10", maxValue: "1000" }), "-1");
    expect(n).toMatchObject({ increment: "-1", start: "100", minValue: "10", maxValue: "1000" });
  });
  it("azalan → artan", () => {
    const n = incrementEditPatch(seq({ increment: "-1", start: "-1", maxValue: "-1" }), "2");
    expect(n).toMatchObject({ increment: "2", start: "1", minValue: null, maxValue: null });
    expect(incrementEditPatch(seq({ increment: "-1", start: "-50", maxValue: "-10" }), "1")).toMatchObject({ start: "-50", maxValue: "-10" });
  });
  it("aynı yön ya da geçersiz/sıfır artış: yalnızca increment değişir", () => {
    expect(incrementEditPatch(seq(), "5")).toEqual(seq({ increment: "5" }));
    expect(incrementEditPatch(seq(), "-")).toEqual(seq({ increment: "-" }));
    expect(incrementEditPatch(seq(), "0")).toEqual(seq({ increment: "0" }));
  });
  it("önceki artış geçersizse (yazım ara durumu) fallbackSign kullanılır", () => {
    expect(incrementEditPatch(seq({ increment: "-" }), "-1", 1)).toMatchObject({ start: "-1", maxValue: "-1" });
    expect(incrementEditPatch(seq({ increment: "-" }), "-1", -1)).toMatchObject({ start: "1", maxValue: null });
  });
  it("dataType bağımsız ve girdi değişmez", () => {
    const d = seq({ dataType: "smallint" });
    const n = incrementEditPatch(d, "-1");
    expect(n.dataType).toBe("smallint");
    expect(d).toEqual(seq({ dataType: "smallint" }));
  });
  it("incrementSign", () => {
    expect(incrementSign("+3")).toBe(1);
    expect(incrementSign("-3")).toBe(-1);
    expect(incrementSign("0")).toBeNull();
    expect(incrementSign("x")).toBeNull();
    expect(incrementSign("99999999999999999999")).toBeNull();
  });
});

describe("sequence notları", () => {
  it("yön/başlangıç/tür değişimi yalnız alter'da (yüklenen tanıma göre)", () => {
    const o = seqObj();
    expect(sequenceDirectionChanged(o, seq({ increment: "-1" }))).toBe(true);
    expect(sequenceDirectionChanged(o, seq({ increment: "7" }))).toBe(false);
    expect(sequenceDirectionChanged(o, seq({ increment: "-" }))).toBe(false);
    expect(sequenceDirectionChanged(null, seq({ increment: "-1" }))).toBe(false);
    expect(sequenceStartChanged(o, seq({ start: "5" }))).toBe(true);
    expect(sequenceStartChanged(o, seq({ increment: "-1", start: "-1" }))).toBe(false);
    expect(sequenceStartChanged(null, seq({ start: "5" }))).toBe(false);
    expect(sequenceTypeChanged(o, seq({ dataType: "integer" }))).toBe(true);
    expect(sequenceTypeChanged(o, seq())).toBe(false);
  });
  it("tür sınırları notu", () => {
    expect(seqTypeBoundsNote("smallint")).toBe(tdes.sequence.bounds("smallint", "-32768", "32767"));
    expect(seqTypeBoundsNote("integer")).toContain("2147483647");
  });
  it("biçim uyarıları: zorunlu boş, biçimsiz, i64 dışı; boş sınırlar serbest", () => {
    expect(sequenceFormatWarnings(seq())).toEqual({});
    expect(sequenceFormatWarnings(seq({ increment: "", start: "1.5", cache: "abc", minValue: "1e3", maxValue: "99999999999999999999" }))).toEqual({
      increment: tdes.sequence.required, start: tdes.sequence.format, cache: tdes.sequence.format,
      minValue: tdes.sequence.format, maxValue: tdes.sequence.format,
    });
    expect(sequenceFormatWarnings(seq({ minValue: "-9223372036854775808" }))).toEqual({});
  });
  it("ownedBy ipucu: farklı şema", () => {
    expect(ownedByHint(seq())).toBeNull();
    expect(ownedByHint(seq({ ownedBy: { schema: "app", table: "t", column: "id" } }))).toBeNull();
    expect(ownedByHint(seq({ ownedBy: { schema: "other", table: "t", column: "id" } }))).toBe(tdes.sequence.ownedBySchemaMismatch);
  });
  it("sequenceSchemaPatch: eski şemayı izleyen OWNED BY birlikte taşınır", () => {
    const owned = { schema: "app", table: "t", column: "id" };
    expect(sequenceSchemaPatch(seq({ ownedBy: owned }), "new")).toMatchObject({ schema: "new", ownedBy: { schema: "new", table: "t", column: "id" } });
    const other = { schema: "x", table: "t", column: "id" };
    expect(sequenceSchemaPatch(seq({ ownedBy: other }), "new").ownedBy).toEqual(other);
    expect(sequenceSchemaPatch(seq(), "new")).toMatchObject({ schema: "new", ownedBy: null });
    const d = seq({ ownedBy: owned });
    sequenceSchemaPatch(d, "new");
    expect(d.ownedBy).toEqual(owned);
  });
  it("sequenceEditPatch / asSequence", () => {
    const d = seqObj();
    expect(sequenceEditPatch(d, { cycle: true })).toEqual({ ...d, cycle: true });
    expect(sequenceEditPatch(schema(), { cycle: true })).toEqual(schema());
    expect(asSequence(d)?.name).toBe("s");
    expect(asSequence(schema())).toBeNull();
  });
});

describe("indeks kolon listesi işlemleri (designer-model yeniden kullanılır)", () => {
  it("ekle/sil/taşı/güncelle değişmez; sınır dışı yok sayılır", () => {
    const d = idx({ columns: [col("a"), col("b")] });
    const added = addColumn(d, newIndexColumn());
    expect(added.columns).toHaveLength(3);
    expect(d.columns).toHaveLength(2);
    expect(removeColumn(d, 0).columns.map((c) => columnKeyLabel(c.key))).toEqual(["b"]);
    expect(removeColumn(d, 5)).toBe(d);
    expect(moveColumn(d, 1, 0).columns.map((c) => columnKeyLabel(c.key))).toEqual(["b", "a"]);
    expect(moveColumn(d, 1, 2)).toBe(d);
    expect(updateColumn(d, 1, { opclass: "text_pattern_ops" }).columns[1]!.opclass).toBe("text_pattern_ops");
    expect(updateColumn(d, -1, { opclass: "x" })).toBe(d);
    expect(updateInclude(idx({ include: ["x", "y"] }), 1, "z").include).toEqual(["x", "z"]);
    const inc = idx({ include: ["x"] });
    expect(updateInclude(inc, 3, "z")).toBe(inc);
  });
  it("columnKeyLabel / switchKeyKind", () => {
    expect(columnKeyLabel({ kind: "column", name: "email" })).toBe("email");
    expect(columnKeyLabel({ kind: "column", name: "" })).toBe(tdes.index.empty);
    expect(columnKeyLabel({ kind: "expression", sql: "lower(\n  email)" })).toBe("lower( email)");
    expect(columnKeyLabel({ kind: "expression", sql: "x".repeat(60) })).toHaveLength(40);
    expect(switchKeyKind({ kind: "column", name: "a" }, "expression")).toEqual({ kind: "expression", sql: "a" });
    expect(switchKeyKind({ kind: "expression", sql: "f(a)" }, "column")).toEqual({ kind: "column", name: "f(a)" });
    const k = { kind: "column", name: "a" } as const;
    expect(switchKeyKind(k, "column")).toBe(k);
  });
  it("nullsFirst üçlü seçici", () => {
    expect(nullsFirstToSelect(null)).toBe("default");
    expect(nullsFirstToSelect(true)).toBe("first");
    expect(nullsFirstToSelect(false)).toBe("last");
    expect(selectToNullsFirst("first")).toBe(true);
    expect(selectToNullsFirst("last")).toBe(false);
    expect(selectToNullsFirst("default")).toBeNull();
    expect(selectToNullsFirst("x")).toBeNull();
  });
});

describe("indeks yöntem kuralları", () => {
  it("btreeOnlyFields / fillfactorEnabled", () => {
    expect(btreeOnlyFields("btree")).toBe(true);
    for (const m of ["hash", "gin", "gist", "brin", "spgist"]) expect(btreeOnlyFields(m)).toBe(false);
    expect(fillfactorEnabled("btree")).toBe(true);
    expect(fillfactorEnabled("gist")).toBe(true);
    expect(fillfactorEnabled("gin")).toBe(false);
    expect(fillfactorEnabled("brin")).toBe(false);
  });
  it("yöntem değişimi yalnızca method'u değiştirir (anahtar tanımı ve fillfactor korunur)", () => {
    const d = idx({ columns: [col("a", { descending: true, nullsFirst: false })], fillfactor: 70 });
    const g = indexMethodPatch(d, "gin");
    expect(g).toEqual({ ...d, method: "gin" });
    expect(g.columns).toBe(d.columns);
    // btree → hash → btree gidiş-dönüşü aynı tanıma döner (farklı indeks yeniden kurulmaz).
    expect(indexMethodPatch(indexMethodPatch(d, "hash"), "btree")).toEqual(d);
    expect(structuralChange(d, indexMethodPatch(indexMethodPatch(d, "brin"), "btree"))).toBe(false);
  });
  it("restrictedControlDisabled: desteklenmeyen seçenek yalnızca varsayılandayken kapalı", () => {
    expect(restrictedControlDisabled(true, true)).toBe(false);
    expect(restrictedControlDisabled(true, false)).toBe(false);
    expect(restrictedControlDisabled(false, true)).toBe(true);
    expect(restrictedControlDisabled(false, false)).toBe(false);
  });
  it("tablespaceChoices: pg_default ve pg_global çıkar; mevcut değer listede yoksa başa", () => {
    const items = [{ value: "pg_default", detail: null }, { value: "pg_global", detail: null }, { value: "fast", detail: null }];
    expect(tablespaceChoices(items, null)).toEqual(["fast"]);
    expect(tablespaceChoices(items, "pg_default")).toEqual(["fast"]);
    expect(tablespaceChoices(items, "fast")).toEqual(["fast"]);
    expect(tablespaceChoices(items, "slow")).toEqual(["slow", "fast"]);
    expect(tablespaceChoices([], null)).toEqual([]);
  });
  it("normalizeTablespaceEdit: temizleme yok, pg_default açıkça seçilir", () => {
    expect(normalizeTablespaceEdit(null, "")).toBeNull();
    expect(normalizeTablespaceEdit("fast", "")).toBe("fast");
    expect(normalizeTablespaceEdit("fast", "  ")).toBe("fast");
    expect(normalizeTablespaceEdit("fast", "pg_default")).toBe("pg_default");
    expect(normalizeTablespaceEdit(null, "fast")).toBe("fast");
  });
  it("fillfactorFromInput: boş → null, ≤3 rakam → sayı, diğerleri yok sayılır (undefined)", () => {
    expect(fillfactorFromInput("")).toBeNull();
    expect(fillfactorFromInput(" 70 ")).toBe(70);
    expect(fillfactorFromInput("100")).toBe(100);
    expect(fillfactorFromInput("5")).toBe(5);
    expect(fillfactorFromInput("7.5")).toBeUndefined();
    expect(fillfactorFromInput("-3")).toBeUndefined();
    expect(fillfactorFromInput("1000")).toBeUndefined();
    expect(fillfactorFromInput("1e3")).toBeUndefined();
    expect(fillfactorFromInput("", true)).toBeUndefined();
    expect(fillfactorFromInput("70", true)).toBeUndefined();
  });
  it("indexReadOnlyReason", () => {
    expect(indexReadOnlyReason(null)).toEqual({ constraint: null, invalid: null });
    expect(indexReadOnlyReason(idx())).toEqual({ constraint: null, invalid: null });
    expect(indexReadOnlyReason(idx({ constraint: "t_pkey" })).constraint).toBe(tdes.index.constraint("t_pkey"));
    expect(indexReadOnlyReason(idx({ constraint: "t_pkey" })).constraint).toContain("'t_pkey'");
    expect(indexReadOnlyReason(idx({ valid: false }))).toEqual({ constraint: null, invalid: tdes.index.invalid });
  });
  it("structuralChange: method/unique/columns/include/whereClause; ad/tablespace/fillfactor/yorum değil", () => {
    const o = idx({ columns: [col("a"), col("b")], include: ["c"] });
    expect(structuralChange(o, structuredClone(o))).toBe(false);
    expect(structuralChange(o, { ...o, name: "x", tablespace: "pg_default", fillfactor: 50, comment: "c" })).toBe(false);
    expect(structuralChange(o, { ...o, method: "hash" })).toBe(true);
    expect(structuralChange(o, { ...o, unique: true })).toBe(true);
    expect(structuralChange(o, { ...o, columns: [col("b"), col("a")] })).toBe(true);
    expect(structuralChange(o, { ...o, columns: [col("a"), col("b", { descending: true })] })).toBe(true);
    expect(structuralChange(o, { ...o, columns: [col("a"), { ...col("b"), key: { kind: "expression", sql: "b" } }] })).toBe(true);
    expect(structuralChange(o, { ...o, columns: [col("a")] })).toBe(true);
    expect(structuralChange(o, { ...o, include: [] })).toBe(true);
    expect(structuralChange(o, { ...o, whereClause: "a > 0" })).toBe(true);
  });
  it("indexEditPatch / asIndex", () => {
    const d: ObjectDef = { kind: "index", ...idx() };
    expect(indexEditPatch(d, { unique: true })).toEqual({ ...d, unique: true });
    expect(indexEditPatch(schema(), { unique: true })).toEqual(schema());
    expect(asIndex(d)?.table).toBe("t");
    expect(asIndex(schema())).toBeNull();
  });
});

describe("alan yolu sözleşmesi (sequence/index)", () => {
  it("sequence: sayılar ve ownedBy.* alanlara; old. ve kind genel", () => {
    const g = groupIssues(
      [
        { path: "start", message: "s" }, { path: "ownedBy.schema", message: "o" }, { path: "ownedBy.column", message: "c" },
        { path: "old.start", message: "eski" }, { path: "kind", message: "k" },
      ],
      SEQUENCE_PATHS,
    );
    expect(issuesFor(g, "start")).toEqual(["s"]);
    expect(issuesFor(g, "ownedBy.schema")).toEqual(["o"]);
    expect(issuesFor(g, "ownedBy")).toEqual(["o", "c"]);
    expect(g.general).toEqual(["eski", "k"]);
  });
  it("index: columns[i].* ve include[i] alanlara; üst-düzey columns, schema/table genel", () => {
    const g = groupIssues(
      [
        { path: "columns[1].key", message: "k1" }, { path: "columns[10].opclass", message: "o10" }, { path: "columns[1]", message: "r1" },
        { path: "columns", message: "genel" }, { path: "include[0]", message: "i0" }, { path: "include", message: "inc" },
        { path: "table", message: "t" }, { path: "constraint", message: "kısıt" },
      ],
      INDEX_PATHS,
    );
    expect(issuesFor(g, "columns[1].key")).toEqual(["k1"]);
    expect(exactIssues(g, "columns[1]")).toEqual(["r1"]);
    expect(issuesFor(g, "columns[1]")).toEqual(["k1", "r1"]);
    expect(issuesFor(g, "columns[10].opclass")).toEqual(["o10"]);
    expect(exactIssues(g, "include")).toEqual(["inc"]);
    expect(exactIssues(g, "include[0]")).toEqual(["i0"]);
    expect(exactIssues(g, "constraint")).toEqual(["kısıt"]);
    expect(g.general).toEqual(["genel", "t"]);
  });
  it("orphanRowIssues: listede olmayan satırların hataları", () => {
    const g = groupIssues([
      { path: "columns[0].key", message: "a" }, { path: "columns[2]", message: "b" }, { path: "columns[3].key", message: "c" },
      { path: "columnsX[5]", message: "x" },
    ]);
    expect(orphanRowIssues(g, "columns", 2)).toEqual(["b", "c"]);
    expect(orphanRowIssues(g, "columns", 4)).toEqual([]);
  });
});

describe("FORM_PATHS (tür → form eşlemesi)", () => {
  it("her ObjectKind için yol kümesi var", () => {
    expect((["schema", "sequence", "extension", "index", "table"] as const).every((k) => FORM_PATHS[k].size > 0)).toBe(true);
    expect(FORM_PATHS.index).toBe(INDEX_PATHS);
    expect(FORM_PATHS.sequence).toBe(SEQUENCE_PATHS);
    expect(FORM_PATHS.table).toBe(TABLE_PATHS);
  });
  it("TABLE_PATHS: satır yolları kendi kökünde, old.* ve üst-düzey columns genel şeritte", () => {
    const g = groupIssues(
      [{ path: "columns[1].dataType", message: "a" }, { path: "columns", message: "b" }, { path: "old.columns[0].attnum", message: "c" }, { path: "partitionBy.keys[0]", message: "d" }],
      TABLE_PATHS,
    );
    expect([...g.byPath.keys()]).toEqual(["columns[1].dataType", "partitionBy.keys[0]"]);
    expect(g.general).toEqual(["b", "c"]);
  });
});

describe("view/matview yolları", () => {
  it("FORM_PATHS.view === VIEW_PATHS; FORM_PATHS.materialized_view === MATVIEW_PATHS", () => {
    expect(FORM_PATHS.view).toBe(VIEW_PATHS);
    expect(FORM_PATHS.materialized_view).toBe(MATVIEW_PATHS);
  });
  it("groupIssues: query alan, old.name ve columns genel", () => {
    const g = groupIssues(
      [{ path: "query", message: "a" }, { path: "old.name", message: "b" }, { path: "columns", message: "c" }] as never,
      VIEW_PATHS,
    );
    expect(g.byPath.get("query")).toEqual(["a"]);
    expect(g.general).toEqual(expect.arrayContaining(["b", "c"]));
    expect(g.general).toHaveLength(2);
  });
});

describe("view/matview onay kutusu yolları alan hatasıdır", () => {
  it("securityBarrier/securityInvoker/recreate (view) ve withData (MV) byPath'e düşer, genel şeride değil", () => {
    const gv = groupIssues(
      [{ path: "securityBarrier", message: "a" }, { path: "securityInvoker", message: "b" }, { path: "recreate", message: "c" }] as never,
      VIEW_PATHS,
    );
    expect(gv.general).toEqual([]);
    expect(issuesFor(gv, "recreate")).toEqual(["c"]);
    const gm = groupIssues([{ path: "withData", message: "d" }] as never, MATVIEW_PATHS);
    expect(gm.general).toEqual([]);
    expect(issuesFor(gm, "withData")).toEqual(["d"]);
  });
});

describe("DATABASE_PATHS", () => {
  it("Plan 15 Task 4 yol kümesi; FORM_PATHS.database aynı", () => {
    expect([...DATABASE_PATHS].sort()).toEqual(
      ["name", "owner", "template", "encoding", "localeProvider", "lcCollate", "lcCtype", "providerLocale", "tablespace", "connectionLimit", "isTemplate", "allowConnections", "settings", "comment"].sort(),
    );
    expect(FORM_PATHS.database).toBe(DATABASE_PATHS);
  });
  it("settings[i].name kökte; force/cascade/old.* genel", () => {
    const g = groupIssues(
      [{ path: "settings[0].name", message: "a" }, { path: "force", message: "b" }, { path: "old.name", message: "c" }] as never,
      DATABASE_PATHS,
    );
    expect(g.byPath.get("settings[0].name")).toEqual(["a"]);
    expect(g.general).toHaveLength(2);
  });
});

describe("ROLE_PATHS", () => {
  it("Plan 15 Task 5 yol kümesi; FORM_PATHS Plan 14 dokuz türü için tanımlı (Plan 18 türleri Plan 19 formlarıyla gelir)", () => {
    expect([...ROLE_PATHS].sort()).toEqual(
      ["name", "login", "superuser", "createdb", "createrole", "inherit", "replication", "bypassrls", "connectionLimit", "validUntil", "password", "memberOf", "settings", "comment"].sort(),
    );
    expect(FORM_PATHS.role).toBe(ROLE_PATHS);
    const kinds = ["schema", "sequence", "extension", "index", "table", "view", "materialized_view", "database", "role", "function", "procedure", "trigger", "type"] as const;
    expect(Object.keys(FORM_PATHS).sort()).toEqual([...kinds].sort());
    expect(kinds.every((k) => FORM_PATHS[k].size > 0)).toBe(true);
  });
  it("memberOf[i].role ve settings[i].value kökte; cascade/old.* genel", () => {
    const g = groupIssues(
      [{ path: "memberOf[1].role", message: "a" }, { path: "settings[0].value", message: "b" }, { path: "cascade", message: "c" }, { path: "old.name", message: "d" }] as never,
      ROLE_PATHS,
    );
    expect(issuesFor(g, "memberOf")).toEqual(["a"]);
    expect(g.byPath.get("settings[0].value")).toEqual(["b"]);
    expect(g.general).toEqual(["c", "d"]);
  });
});

describe("ROUTINE_PATHS (Plan 19 Task 3)", () => {
  it("spec §8.5 kök anahtarları; FORM_PATHS.function === FORM_PATHS.procedure === ROUTINE_PATHS", () => {
    expect([...ROUTINE_PATHS].sort()).toEqual(
      ["schema", "name", "owner", "comment", "language", "body", "cost", "rows", "volatility", "strict", "leakproof", "parallel", "kind", "object", "args", "returns", "settings", "recreate", "securityDefiner"].sort(),
    );
    expect(FORM_PATHS.function).toBe(ROUTINE_PATHS);
    expect(FORM_PATHS.procedure).toBe(ROUTINE_PATHS);
  });
  it("groupIssues: args[1].typeName / returns.columns[0].name / settings[0].value alan; old.args[0].typeName ve bilinmeyen genel", () => {
    const g = groupIssues(
      [
        { path: "args[1].typeName", message: "a" }, { path: "returns.columns[0].name", message: "b" }, { path: "settings[0].value", message: "c" },
        { path: "old.args[0].typeName", message: "d" }, { path: "securityDefinerX", message: "e" }, { path: "securityDefiner", message: "g" }, { path: "args", message: "f" },
      ] as never,
      ROUTINE_PATHS,
    );
    expect(g.byPath.get("args[1].typeName")).toEqual(["a"]);
    expect(issuesFor(g, "returns")).toEqual(["b"]);
    expect(issuesFor(g, "args")).toEqual(["a", "f"]);
    expect(g.byPath.get("settings[0].value")).toEqual(["c"]);
    expect(g.general).toEqual(["d", "e"]);
    expect(g.byPath.get("securityDefiner")).toEqual(["g"]);
  });
});

describe("TRIGGER_PATHS (Plan 19 Task 4)", () => {
  it("spec §8.5 kök anahtarları; FORM_PATHS.trigger === TRIGGER_PATHS", () => {
    expect([...TRIGGER_PATHS].sort()).toEqual(
      ["schema", "table", "name", "timing", "events", "level", "when", "functionSchema", "functionName", "args", "oldTable", "newTable", "deferrable", "initiallyDeferred", "comment", "object"].sort(),
    );
    expect(FORM_PATHS.trigger).toBe(TRIGGER_PATHS);
  });
  it("groupIssues: events.updateColumns[1] / events.truncate / args[0] alan; old.table ve bilinmeyen genel", () => {
    const g = groupIssues(
      [
        { path: "events.updateColumns[1]", message: "a" }, { path: "events.truncate", message: "b" }, { path: "args[0]", message: "c" },
        { path: "old.table", message: "d" }, { path: "enabledX", message: "e" }, { path: "functionName", message: "f" },
      ] as never,
      TRIGGER_PATHS,
    );
    expect(issuesFor(g, "events")).toEqual(["a", "b"]);
    expect(g.byPath.get("args[0]")).toEqual(["c"]);
    expect(g.byPath.get("functionName")).toEqual(["f"]);
    expect(g.general).toEqual(["d", "e"]);
  });
});

describe("TYPE_PATHS (Plan 19 Task 5)", () => {
  it("spec §8.5 kök anahtarları; FORM_PATHS.type === TYPE_PATHS", () => {
    expect([...TYPE_PATHS].sort()).toEqual(["schema", "name", "owner", "comment", "body", "object"].sort());
    expect(FORM_PATHS.type).toBe(TYPE_PATHS);
  });
  it("groupIssues: body.values[2].label doğru satıra; liste düzeyi body.values/body.attributes ayrı; old.* genel", () => {
    const g = groupIssues(
      [
        { path: "body.values[2].label", message: "a" }, { path: "body.values", message: "b" }, { path: "body.values[12].original", message: "c" },
        { path: "body.attributes[0].typeName", message: "d" }, { path: "body.attributes", message: "e" }, { path: "body.subtype", message: "f" },
        { path: "old.body.values[0].label", message: "g" }, { path: "bodyX", message: "h" }, { path: "object", message: "i" },
      ] as never,
      TYPE_PATHS,
    );
    expect(issuesFor(g, "body.values[2]")).toEqual(["a"]);
    expect(issuesFor(g, "body.values[1]")).toEqual([]);
    expect(exactIssues(g, "body.values")).toEqual(["b"]);
    expect(orphanRowIssues(g, "body.values", 3)).toEqual(["c"]);
    expect(g.byPath.get("body.attributes[0].typeName")).toEqual(["d"]);
    expect(exactIssues(g, "body.attributes")).toEqual(["e"]);
    expect(g.byPath.get("body.subtype")).toEqual(["f"]);
    expect(g.byPath.get("object")).toEqual(["i"]);
    expect(g.general).toEqual(["g", "h"]);
  });
});
