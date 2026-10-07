// Table formu durumu: odak dağıtıcı ve SelfSnap korumalı düzenleme. Sinyaller gerçek olsun diye solid-js tarayıcı
// yapısına yönlendirilir (vitest "node" ortamı sunucu yapısına çözer).
import { describe, expect, it, vi } from "vitest";
import type { ConstraintDef } from "../../ipc/bindings/ConstraintDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TableDef } from "../../ipc/bindings/TableDef";
import type { FocusRequest } from "./designer-controller";

vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));

const T = (p: Partial<TableDef> = {}): ObjectDef => ({
  kind: "table", schema: "public", name: "t", owner: null, tablespace: null, persistence: "permanent", columns: [], constraints: [],
  partitionBy: null, fillfactor: null, comment: null, partitionText: null, isPartition: false, inherits: false, unsupported: [], ...p,
});
const col = (name: string, attnum: number | null) => ({
  attnum, name, dataType: "integer", notNull: false, default: null, identity: null, generated: null, collation: null, comment: null, using: null,
});
const fk = (refTable: string, oid: number | null): ConstraintDef => ({
  oid, name: null, deferrable: false, initiallyDeferred: false, notValid: false,
  kind: { type: "foreign_key", columns: ["a"], refSchema: "public", refTable, refColumns: ["a"], matchType: "simple", onUpdate: "no_action", onDelete: "no_action" },
});

async function setup(initial: ObjectDef) {
  const solid = await import("solid-js");
  const { createTableFormState } = await import("./table-form-state");
  const { renameTable } = await import("./table-model");
  const { removeConstraintSnap } = await import("./table-constraints");
  const [draft, setDraft] = solid.createSignal<ObjectDef | null>(initial);
  const [status, setStatus] = solid.createSignal<"loading" | "ready">("ready");
  const [focus, setFocus] = solid.createSignal<FocusRequest | null>(null);
  let editable = true;
  const consumed: number[] = [];
  const designer = {
    model: () => ({ draft: draft() }),
    focusRequest: () => focus(),
    consumeFocus: (_: string, seq: number) => {
      consumed.push(seq);
      if (focus()?.seq === seq) setFocus(null);
    },
    edit: (_: string, f: (d: ObjectDef) => ObjectDef | void) => {
      const d = draft();
      if (!editable || !d) return;
      const copy = structuredClone(d);
      setDraft(structuredClone(f(copy) ?? copy)); // denetleyici gibi: dönen nesne kopyalanır
    },
  };
  const props = {
    designer: designer as never,
    tabId: "T1",
    model: { get status() { return status(); }, get draft() { return draft(); } },
  };
  return {
    solid, props, createTableFormState, renameTable, removeConstraintSnap, draft, setDraft, setStatus, setFocus, consumed,
    setEditable: (v: boolean) => (editable = v),
  };
}

describe("createTableFormState", () => {
  it("kolon/kısıt odağı: bölüm + satır vurgusu, istek tüketilir; bulunamayan satır → index null", async () => {
    const s = await setup(T({ columns: [col("a", 1), col("b", 2)], constraints: [fk("x", 77)] }));
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    {
      expect(st.section()).toBe("general");
      s.setFocus({ field: "column:2", seq: 1 });
      expect([st.section(), st.highlight()]).toEqual(["columns", { section: "columns", index: 1, seq: 1 }]);
      expect(s.consumed).toEqual([1]);
      s.setFocus({ field: "constraint:99", seq: 2 });
      expect([st.section(), st.highlight()]).toEqual(["constraints", { section: "constraints", index: null, seq: 2 }]);
      s.setFocus({ field: "constraint:77", seq: 3 });
      expect(st.highlight()).toEqual({ section: "constraints", index: 0, seq: 3 });
      dispose();
    }
  });

  it("ad odağı Genel'e geçer ama tüketmez (alanı TableGeneral odaklar); yükleme bitene dek bekler", async () => {
    const s = await setup(T({ columns: [col("a", 1)] }));
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    {
      s.setStatus("loading");
      s.setFocus({ field: "column:1", seq: 5 });
      expect([st.section(), s.consumed]).toEqual(["general", []]);
      s.setStatus("ready");
      expect([st.section(), s.consumed]).toEqual(["columns", [5]]);
      s.setFocus({ field: "name", seq: 6 });
      expect([st.section(), s.consumed]).toEqual(["general", [5]]);
      dispose();
    }
  });

  it("anlık görüntü düzenlemeler boyunca korunur; ara ad başka tablonun FK'sini çalmaz", async () => {
    const s = await setup(T({ constraints: [fk("t", 1), fk("other", 2)] }));
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    {
      st.editor.editSnap((d, snap) => ({ d: s.renameTable(d, { name: "other" }, snap), snap }));
      st.editor.edit((d) => ({ ...d, comment: "c" })); // anlık görüntüsüz düzenleme de kayıtlı (dış değişim sayılmaz)
      st.editor.editSnap((d, snap) => ({ d: s.renameTable(d, { name: "t3" }, snap), snap }));
      const d = s.draft() as TableDef;
      expect(d.constraints.map((c) => (c.kind.type === "foreign_key" ? c.kind.refTable : null))).toEqual(["t3", "other"]);
      expect(d.comment).toBe("c");
      dispose();
    }
  });

  it("sarmalayıcı sonucu kaydedilir; dış taslak değişimi (yeniden yükleme) anlık görüntüyü yeniden alır; red değiştirmez", async () => {
    const s = await setup(T({ constraints: [fk("x", 1), fk("t", 2)] }));
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    {
      st.editor.editSnap((d, snap) => s.removeConstraintSnap(d, 0, snap)); // kendine-başvuru 1 → 0
      st.editor.editSnap((d, snap) => ({ d: s.renameTable(d, { name: "n" }, snap), snap }));
      expect(((s.draft() as TableDef).constraints[0]!.kind as { refTable: string }).refTable).toBe("n");
      // Yeniden yükleme: yüklenen tanımda kendine-başvuru yok → ad değişimi hiçbir FK'ye yazmaz.
      s.setDraft(T({ name: "n", constraints: [fk("n2", 5)] }));
      st.editor.editSnap((d, snap) => ({ d: s.renameTable(d, { name: "n2" }, snap), snap }));
      expect(((s.draft() as TableDef).constraints[0]!.kind as { refTable: string }).refTable).toBe("n2");
      // Salt okunur: geri çağrı çalışmaz, taslak aynı.
      s.setEditable(false);
      const before = s.draft();
      st.editor.edit((d) => ({ ...d, comment: "x" }));
      expect(s.draft()).toBe(before);
      dispose();
    }
  });

  it("M-1: bekleyen ad isteği kullanıcıyı yalnızca bir kez Genel'e çeker (kilitli sekmede istek beklerken yeniden çalışmalar çekmez)", async () => {
    const s = await setup(T({ columns: [col("a", 1)] }));
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    s.setFocus({ field: "name", seq: 1 });
    expect(st.section()).toBe("general");
    st.setSection("columns");
    // Dağıtıcıyı yeniden çalıştıran değişimler: yeniden yükleme (status + taslak) ve kendi düzenlemesi.
    s.setStatus("loading");
    s.setStatus("ready");
    s.setDraft(T({ columns: [col("a", 1)] }));
    st.editor.edit((d) => ({ ...d, comment: "c" }));
    expect([st.section(), s.consumed]).toEqual(["columns", []]);
    // Yeni bir ad isteği yine Genel'e geçer.
    s.setFocus({ field: "name", seq: 2 });
    expect(st.section()).toBe("general");
    dispose();
  });

  it("M-2: vurgu ve 'satır yok' şeridi taslak dışarıdan değişince temizlenir; formun kendi düzenlemesi temizlemez", async () => {
    const s = await setup(T({ columns: [col("a", 1)], constraints: [fk("x", 7)] }));
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    s.setFocus({ field: "constraint:99", seq: 1 });
    expect(st.highlight()).toEqual({ section: "constraints", index: null, seq: 1 });
    st.editor.edit((d) => ({ ...d, comment: "c" }));
    st.editor.editSnap((d, snap) => s.removeConstraintSnap(d, 0, snap));
    expect(st.highlight()).toEqual({ section: "constraints", index: null, seq: 1 });
    s.setDraft(T({ columns: [col("a", 1)] })); // yeniden yükleme
    expect(st.highlight()).toBeNull();
    // Yükleme ile aynı anda gelen yeni istek yine uygulanır.
    s.setStatus("loading");
    s.setFocus({ field: "column:1", seq: 2 });
    s.solid.batch(() => {
      s.setDraft(T({ columns: [col("a", 1)] }));
      s.setStatus("ready");
    });
    expect(st.highlight()).toEqual({ section: "columns", index: 0, seq: 2 });
    dispose();
  });

  it("C-2: editor.edit kısıt sayısını değiştirirse (DEV) atar; taslak değişmez", async () => {
    const s = await setup(T({ constraints: [fk("x", 1)] }));
    const { SNAP_GUARD } = await import("./table-form-state");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const before = s.draft();
    expect(() => st.editor.edit((d) => ({ ...d, constraints: [] }))).toThrow(SNAP_GUARD);
    expect(s.draft()).toBe(before);
    dispose();
  });

  it("Task 7: editor.snap() güncel anlık görüntüyü verir ve taslağa bağlı memo onu düzenleme/yeniden yükleme sonrası yeniden okur", async () => {
    const s = await setup(T({ constraints: [fk("x", 1), fk("t", 2)] }));
    const { withFkTargetSnap } = await import("./table-constraints");
    const [res, dispose] = s.solid.createRoot((d) => {
      const st = s.createTableFormState(s.props);
      const seen = s.solid.createMemo(() => (s.draft(), [...st.editor.snap()]));
      return [{ st, seen }, d] as const;
    });
    const { st, seen } = res;
    expect(seen()).toEqual([1]);
    // FK 0 tablonun kendisine yöneltilir (taslak adı "t"): her iki FK kendine.
    st.editor.editSnap((d, snap) => withFkTargetSnap(d, 0, { refTable: "t" }, snap));
    expect(seen()).toEqual([0, 1]);
    // Ara ad: yalnızca kayıttakiler taşınır; snap yeniden tespit edilmez.
    st.editor.editSnap((d, snap) => ({ d: s.renameTable(d, { name: "x" }, snap), snap }));
    expect(seen()).toEqual([0, 1]);
    // Yeniden yükleme: snap yeni taslaktan alınır.
    s.setDraft(T({ name: "n", constraints: [fk("other", 5)] }));
    expect(seen()).toEqual([]);
    dispose();
  });

  const pkC = (columns: string[], oid: number): ConstraintDef => ({
    oid, name: null, deferrable: false, initiallyDeferred: false, notValid: false, kind: { type: "primary_key", columns, include: [] },
  });
  const uqC = (columns: string[], oid: number): ConstraintDef => ({
    oid, name: null, deferrable: false, initiallyDeferred: false, notValid: false, kind: { type: "unique", columns, include: [], nullsNotDistinct: false },
  });
  const listsOf = (d: TableDef) => d.constraints.map((c) => (c.kind as { columns: string[] }).columns);

  it("createColumnRenames: ara ad başka kolonun başvurusunu çalmaz; kendine-başvuru hedefi ve PK taşınır", async () => {
    const self: ConstraintDef = {
      ...fk("t", 3), kind: { type: "foreign_key", columns: ["i"], refSchema: "public", refTable: "t", refColumns: ["id"], matchType: "simple", onUpdate: "no_action", onDelete: "no_action" },
    };
    const s = await setup(T({ columns: [{ ...col("id", 1), notNull: true }, col("i", 2)], constraints: [pkC(["id"], 1), uqC(["i"], 2), self] }));
    const { createColumnRenames } = await import("./table-form-state");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const r = createColumnRenames(st.editor, () => null);
    r.rename(0, "i"); // ara ad = 2. kolonun adı
    r.rename(0, "ix");
    let d = s.draft() as TableDef;
    expect(d.columns.map((c) => c.name)).toEqual(["ix", "i"]);
    expect(listsOf(d)).toEqual([["ix"], ["i"], ["i"]]);
    expect((d.constraints[2]!.kind as { refColumns: string[] }).refColumns).toEqual(["ix"]);
    r.rename(1, "j"); // ikinci kolon yalnızca kendi başvurularını taşır
    d = s.draft() as TableDef;
    expect(listsOf(d)).toEqual([["ix"], ["j"], ["j"]]);
    dispose();
  });

  it("I-1: ad değişimi NOT NULL'a dokunmaz (PK(id) + nullable x → ara 'id' → 'idx' notNull false kalır)", async () => {
    const s = await setup(T({ columns: [{ ...col("id", 1), notNull: true }, col("x", 2)], constraints: [pkC(["id"], 1)] }));
    const { createColumnRenames } = await import("./table-form-state");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const r = createColumnRenames(st.editor, () => null);
    for (const v of ["i", "id", "idx"]) r.rename(1, v);
    const d = s.draft() as TableDef;
    expect(d.columns[1]).toMatchObject({ name: "idx", notNull: false });
    expect(listsOf(d)).toEqual([["id"]]);
    dispose();
  });

  it("I-2: başvurular odak oturumları arasında korunur (x'e 'i' yaz, odaktan çık, dön, 'x' yaz → UNIQUE(x), UNIQUE(i))", async () => {
    const s = await setup(T({ columns: [col("x", 1), col("i", 2)], constraints: [uqC(["x"], 1), uqC(["i"], 2)] }));
    const { createColumnRenames } = await import("./table-form-state");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const r = createColumnRenames(st.editor, () => null);
    r.rename(0, "i");
    st.editor.edit((d) => ({ ...d, columns: d.columns.map((c, k) => (k === 1 ? { ...c, dataType: "text" } : c)) })); // başka alan düzenlemesi
    r.rename(0, "x");
    expect(listsOf(s.draft() as TableDef)).toEqual([["x"], ["i"]]);
    // Dış değişim (yeniden yükleme) kaydı düşürür: yeni taslağın adlarından yeniden alınır.
    s.setDraft(T({ columns: [col("a", 1), col("b", 2)], constraints: [uqC(["b"], 1), uqC(["a"], 2)] }));
    r.rename(0, "a2");
    expect(listsOf(s.draft() as TableDef)).toEqual([["b"], ["a2"]]);
    dispose();
  });

  it("M-1: yeniden adlandırılmış yüklü kolonu silmek önce özgün adı geri yazar; geri alma birebir yüklenen tanım", async () => {
    const { defsEqual } = await import("./designer-model");
    const { restoreColumn } = await import("./table-model");
    const loaded = T({ columns: [col("x", 1), col("y", 2)], constraints: [uqC(["x"], 1)] });
    const s = await setup(loaded);
    const { createColumnRenames } = await import("./table-form-state");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const original = structuredClone(loaded) as TableDef;
    const r = createColumnRenames(st.editor, () => original);
    r.rename(0, "z");
    expect(listsOf(s.draft() as TableDef)).toEqual([["z"]]);
    r.remove(0);
    let d = s.draft() as TableDef;
    expect(d.columns.map((c) => c.name)).toEqual(["y"]);
    expect(listsOf(d)).toEqual([["x"]]);
    st.editor.edit((x) => restoreColumn(x, original, 1));
    d = s.draft() as TableDef;
    expect(defsEqual(d as ObjectDef, loaded)).toBe(true);
    dispose();
  });
  // Final inceleme I-1: PK'nın zorladığı NOT NULL, kolon PK'dan çıkınca eski değerine döner (dene-geri al = değişiklik yok).
  async function pkSetup(loaded: ObjectDef) {
    const s = await setup(loaded);
    const K = await import("./table-constraints");
    const M = await import("./table-model");
    const { defsEqual } = await import("./designer-model");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const editC = (i: number, f: (c: ConstraintDef) => ConstraintDef) => st.editor.edit((x) => K.withPrimaryKeyNotNull(K.updateConstraint(x, i, f)));
    const notNull = () => (s.draft() as TableDef).columns.map((c) => c.notNull);
    return { s, K, M, st, dispose, editC, notNull, same: () => defsEqual(s.draft()!, loaded) };
  }
  const orders = () => T({ name: "orders", columns: [{ ...col("id", 1), notNull: true }, col("code", 2)], constraints: [pkC(["id"], 100), uqC(["code"], 101)] });

  it("final I-1 S44: yüklü PK'ya nullable kolon ekle-çıkar → NOT NULL geri döner, taslak yüklenenle aynı", async () => {
    const loaded = orders();
    const t = await pkSetup(loaded);
    t.editC(0, (c) => t.K.addConstraintColumn(c, "columns", "code"));
    expect(t.notNull()).toEqual([true, true]);
    t.editC(0, (c) => t.K.removeConstraintColumn(c, "columns", 1));
    expect(t.notNull()).toEqual([true, false]);
    expect(t.same()).toBe(true);
    t.dispose();
  });

  it("final I-1 S42: PK sil, yeni PK(code), çipi çıkar, yeni PK'yı sil, yüklü PK'yı geri al → değişiklik yok", async () => {
    const loaded = orders();
    const original = structuredClone(loaded) as TableDef;
    const t = await pkSetup(loaded);
    t.st.editor.editSnap((d, snap) => t.K.removeConstraintSnap(d, 0, snap));
    t.st.editor.editSnap((d, snap) => t.K.addConstraintSnap(d, "primary_key", snap));
    t.editC(1, (c) => t.K.addConstraintColumn(c, "columns", "code"));
    expect(t.notNull()).toEqual([true, true]);
    t.editC(1, (c) => t.K.removeConstraintColumn(c, "columns", 0));
    expect(t.notNull()).toEqual([true, false]);
    t.editC(1, (c) => t.K.addConstraintColumn(c, "columns", "code")); // yeniden ekle → kart silinince de geri dönmeli
    t.st.editor.editSnap((d, snap) => t.K.removeConstraintSnap(d, 1, snap));
    expect(t.notNull()).toEqual([true, false]);
    t.st.editor.editSnap((d, snap) => t.K.restoreConstraintWithPk(d, original, 100, snap));
    expect(t.same()).toBe(true);
    t.dispose();
  });

  it("final I-1: yeni kolon PK'ya eklenip çıkarılınca NOT NULL false'a döner; kullanıcının işaretlediği NOT NULL korunur", async () => {
    const t = await pkSetup(T({ name: "n", columns: [col("a", 1)], constraints: [] }));
    t.st.editor.edit(t.M.addTableColumn);
    t.st.editor.edit((x) => t.M.updateTableColumn(x, 1, (c) => ({ ...c, name: "n1", dataType: "integer" })));
    t.st.editor.edit(t.M.addTableColumn);
    t.st.editor.edit((x) => t.M.updateTableColumn(x, 2, (c) => ({ ...c, name: "n2", dataType: "integer", notNull: true }))); // kullanıcı seçimi
    t.st.editor.editSnap((d, snap) => t.K.addConstraintSnap(d, "primary_key", snap));
    for (const n of ["n1", "n2", "a"]) t.editC(0, (c) => t.K.addConstraintColumn(c, "columns", n));
    expect(t.notNull()).toEqual([true, true, true]);
    t.editC(0, (c) => t.K.removeConstraintColumn(c, "columns", 0)); // n1 çıkar
    expect(t.notNull()).toEqual([true, false, true]);
    t.st.editor.editSnap((d, snap) => t.K.removeConstraintSnap(d, 0, snap)); // PK sil: a (yüklü, zorlanmış) false'a, n2 (kullanıcı) true kalır
    expect(t.notNull()).toEqual([false, false, true]);
    // Yeni kolon silinince sıralar kayar: n2 zorlanmış değil, n1 silinirse kayıt yine doğru kolona ait.
    t.st.editor.editSnap((d, snap) => t.K.addConstraintSnap(d, "primary_key", snap));
    t.editC(0, (c) => t.K.addConstraintColumn(c, "columns", "n1"));
    t.st.editor.edit((x) => t.M.removeTableColumn(x, 2)); // n2 silinir (n1 zorlanmış kalır)
    t.editC(0, (c) => t.K.removeConstraintColumn(c, "columns", 0));
    expect(t.notNull()).toEqual([false, false]);
    t.dispose();
  });

  it("final I-1: yeniden yükleme zorlanmış kaydını sıfırlar (yüklenen NOT NULL kolon PK'dan çıkınca NOT NULL kalır)", async () => {
    const t = await pkSetup(orders());
    t.editC(0, (c) => t.K.addConstraintColumn(c, "columns", "code"));
    // Uygulama sonrası yeniden yükleme: code artık yüklü NOT NULL.
    t.s.setDraft(T({ name: "orders", columns: [{ ...col("id", 1), notNull: true }, { ...col("code", 2), notNull: true }], constraints: [pkC(["id", "code"], 100)] }));
    t.editC(0, (c) => t.K.removeConstraintColumn(c, "columns", 1));
    expect(t.notNull()).toEqual([true, true]);
    t.dispose();
  });
});
