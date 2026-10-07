import { describe, expect, it } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { ROOT, emptyTree, type TreeState } from "../explorer/tree-model";
import type { AppliedInfo } from "./designer-controller";
import { extraRefreshIds, titleAfterApply, type OpenContext } from "./refresh-model";

const flags = { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
const tbl = (p: Record<string, unknown> = {}): ObjectDef =>
  ({ kind: "table", schema: "public", name: "orders", owner: null, tablespace: null, persistence: "permanent", columns: [], constraints: [], partitionBy: null, fillfactor: null, comment: null, partitionText: null, isPartition: false, inherits: false, unsupported: [], ...p }) as ObjectDef;
const seq = (p: Record<string, unknown> = {}): ObjectDef => ({ kind: "sequence", schema: "public", name: "s", ...p }) as unknown as ObjectDef;
const info = (p: Partial<AppliedInfo> & Pick<AppliedInfo, "after">): AppliedInfo => ({
  tabId: "T", kind: p.after.kind, mode: "alter", before: null, object: { kind: p.after.kind, oid: 1 }, ...p,
});

/** db → Schemas → public/other → Tables/Sequences; tablo → Columns/Constraints/Indexes/Triggers. `loaded`: yüklü (children girdisi olan) düğümler. */
function tree(loaded: string[] = []): TreeState {
  const s = emptyTree();
  const add = (id: string, parent: string, kind: NodeKind, label: string) => {
    s.nodes.set(id, { id, kind, label, detail: null, hasChildren: false, serverId: null, flags, color: null, tag: null } as TreeNode);
    s.parent.set(id, parent);
    s.children.set(parent, [...(s.children.get(parent) ?? []), id]);
  };
  add("db", ROOT, "database", "app");
  add("sc", "db", "schemaCollection", "Schemas");
  add("pub", "sc", "schema", "public");
  add("oth", "sc", "schema", "other");
  add("pub-t", "pub", "tableCollection", "Tables");
  add("oth-t", "oth", "tableCollection", "Tables");
  add("oth-s", "oth", "sequenceCollection", "Sequences");
  add("tbl", "pub-t", "table", "orders");
  add("cols", "tbl", "columnCollection", "Columns");
  add("cons", "tbl", "constraintCollection", "Constraints");
  add("idx", "tbl", "indexCollection", "Indexes");
  add("trg", "tbl", "triggerCollection", "Triggers");
  for (const id of loaded) if (!s.children.has(id)) s.children.set(id, []);
  return s;
}
const CTX: OpenContext = { databaseNodeId: "db", tableNodeId: "tbl" };

describe("refresh-model", () => {
  it("titleAfterApply: create → 'Düzenle: <ad>'; alter ad değişimi → yeni ad; şema değişimi → başlık (ad aynı); değişiklik yok → null", () => {
    expect(titleAfterApply(info({ mode: "create", before: null, after: tbl({ name: "x" }) }))).toBe(tdes.tab.edit("Table", "x"));
    expect(titleAfterApply(info({ before: tbl(), after: tbl({ name: "y" }) }))).toBe(tdes.tab.edit("Table", "y"));
    expect(titleAfterApply(info({ before: tbl(), after: tbl({ schema: "other" }) }))).toBe(tdes.tab.edit("Table", "orders"));
    expect(titleAfterApply(info({ before: tbl(), after: tbl({ comment: "c" }) }))).toBeNull();
    expect(titleAfterApply(info({ before: { kind: "schema", name: "a", owner: null, comment: null }, after: { kind: "schema", name: "a", owner: "x", comment: null } }))).toBeNull();
  });

  it("extraRefreshIds: tablo düğümünün yalnızca YÜKLÜ Columns/Constraints/Indexes koleksiyonları; triggers hariç", () => {
    const a = info({ before: tbl(), after: tbl({ comment: "c" }) });
    expect(extraRefreshIds(tree(["cons", "trg"]), CTX, a, null)).toEqual(["pub-t", "cons"]);
    expect(extraRefreshIds(tree(["cols", "cons", "idx", "trg"]), CTX, a, null)).toEqual(["pub-t", "cols", "cons", "idx"]);
    expect(extraRefreshIds(tree(), CTX, a, null)).toEqual(["pub-t"]);
  });

  it("extraRefreshIds: sequence/table şema taşımada yeni şemanın yüklü koleksiyonu; şema/koleksiyon yoksa ya da yüklü değilse eklenmez", () => {
    const move = info({ before: tbl(), after: tbl({ schema: "other" }) });
    expect(extraRefreshIds(tree(["oth-t"]), CTX, move, null)).toEqual(["pub-t", "oth-t"]);
    expect(extraRefreshIds(tree(), CTX, move, null)).toEqual(["pub-t"]);
    expect(extraRefreshIds(tree(["oth-t"]), CTX, info({ before: tbl(), after: tbl({ schema: "yok" }) }), null)).toEqual(["pub-t"]);
    const sm = info({ before: seq(), after: seq({ schema: "other" }) });
    expect(extraRefreshIds(tree(["oth-s", "oth-t"]), { databaseNodeId: "db", tableNodeId: null }, sm, null)).toEqual(["oth-s"]);
  });

  it("extraRefreshIds: kolon/kısıt düğümünden açılışta (birincil = tablo) üst Tables koleksiyonu ve eski/yeni şema yenilenir; yüklü değilse eklenmez", () => {
    const rename = info({ before: tbl(), after: tbl({ name: "orders2" }) });
    expect(extraRefreshIds(tree(["cols"]), CTX, rename, "tbl")).toEqual(["pub-t", "cols"]);
    const t = tree();
    t.children.delete("pub-t");
    expect(extraRefreshIds(t, CTX, rename, "tbl")).toEqual([]);
    expect(extraRefreshIds(tree(["oth-t"]), CTX, info({ before: tbl(), after: tbl({ schema: "other" }) }), "tbl")).toEqual(["pub-t", "oth-t"]);
  });

  it("extraRefreshIds: create'ten dönüşen sekme (tableNodeId yok): koleksiyondan etiketle tablo bulunur; Columns/Constraints/Indexes + Tables yenilenir", () => {
    const ctx: OpenContext = { databaseNodeId: "db", tableNodeId: null, collectionNodeId: "pub-t" };
    const t = tree(["cols", "idx"]);
    expect(extraRefreshIds(t, ctx, info({ before: tbl(), after: tbl({ comment: "c" }) }), null)).toEqual(["pub-t", "cols", "idx"]);
    // yeniden adlandırma: ağaçta hâlâ eski etiket
    expect(extraRefreshIds(t, ctx, info({ before: tbl(), after: tbl({ name: "yeni" }) }), "pub-t")).toEqual(["cols", "idx"]);
    // düğüm henüz ağaçta yok → tablo kuralı eklenmez; şema taşıma kuralı yine çalışır
    expect(extraRefreshIds(t, ctx, info({ before: tbl({ name: "baska" }), after: tbl({ name: "yok2" }) }), null)).toEqual([]);
    expect(extraRefreshIds(tree(["oth-t"]), ctx, info({ before: tbl(), after: tbl({ schema: "other" }) }), "pub-t")).toEqual(["oth-t"]);
  });

  it("extraRefreshIds: ctx null ya da tablo düğümü ağaçta yoksa yalnızca şema kuralı; sonuç tekrarsız ve primary hariç", () => {
    const move = info({ before: tbl(), after: tbl({ schema: "other" }) });
    const t = tree(["cols", "oth-t"]);
    expect(extraRefreshIds(t, null, move, null)).toEqual([]);
    expect(extraRefreshIds(t, { databaseNodeId: "db", tableNodeId: "yok" }, move, null)).toEqual(["oth-t"]);
    expect(extraRefreshIds(t, CTX, move, "oth-t")).toEqual(["pub-t", "cols"]);
    expect(extraRefreshIds(t, CTX, move, "pub-t")).toEqual(["cols", "oth-t"]);
  });
});
