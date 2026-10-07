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
});
