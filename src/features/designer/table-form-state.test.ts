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

  it("createColumnRenamer: başvurular odak oturumunun ilk vuruşunda alınır; ara ad başka kolonun başvurusunu çalmaz; PK NOT NULL", async () => {
    const pkC = (columns: string[], oid: number): ConstraintDef => ({
      oid, name: null, deferrable: false, initiallyDeferred: false, notValid: false, kind: { type: "primary_key", columns, include: [] },
    });
    const uqC = (columns: string[], oid: number): ConstraintDef => ({
      oid, name: null, deferrable: false, initiallyDeferred: false, notValid: false, kind: { type: "unique", columns, include: [], nullsNotDistinct: false },
    });
    const self: ConstraintDef = {
      ...fk("t", 3), kind: { type: "foreign_key", columns: ["i"], refSchema: "public", refTable: "t", refColumns: ["id"], matchType: "simple", onUpdate: "no_action", onDelete: "no_action" },
    };
    const s = await setup(T({ columns: [col("id", 1), col("i", 2)], constraints: [pkC(["id"], 1), uqC(["i"], 2), self] }));
    const { createColumnRenamer } = await import("./table-form-state");
    const [st, dispose] = s.solid.createRoot((d) => [s.createTableFormState(s.props), d] as const);
    const r = createColumnRenamer(st.editor, () => 0);
    r.begin();
    r.input("i"); // ara ad = 2. kolonun adı
    r.input("ix");
    r.end();
    let d = s.draft() as TableDef;
    expect(d.columns.map((c) => c.name)).toEqual(["ix", "i"]);
    expect(d.constraints.map((c) => (c.kind as { columns: string[] }).columns)).toEqual([["ix"], ["i"], ["i"]]);
    expect((d.constraints[2]!.kind as { refColumns: string[] }).refColumns).toEqual(["ix"]); // kendine-başvuru hedefi
    expect(d.columns[0]!.notNull).toBe(true); // PK üyesi
    // Yeni oturum: ikinci kolonu yeniden adlandırmak yalnızca onun başvurularını taşır.
    const r2 = createColumnRenamer(st.editor, () => 1);
    r2.begin();
    r2.input("j");
    d = s.draft() as TableDef;
    expect(d.constraints.map((c) => (c.kind as { columns: string[] }).columns)).toEqual([["ix"], ["j"], ["j"]]);
    dispose();
  });
});
