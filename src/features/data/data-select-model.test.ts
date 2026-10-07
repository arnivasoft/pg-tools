import { describe, expect, it } from "vitest";
import type { OrderBy } from "../../ipc/bindings/OrderBy";
import { EMPTY_SELECT, MAX_ORDER, cycleOrder, selectErrors, toRequest } from "./data-select-model";

const asc = (column: string): OrderBy => ({ column, desc: false });
const desc = (column: string): OrderBy => ({ column, desc: true });

describe("data-select-model", () => {
  it("EMPTY_SELECT: sıralama yok, filtre boş", () => {
    expect(EMPTY_SELECT).toEqual({ order: [], filter: "" });
    expect(MAX_ORDER).toBe(8);
  });

  it("cycleOrder: yok → artan → azalan → yok", () => {
    const a = cycleOrder([], "id", false);
    expect(a).toEqual([asc("id")]);
    const b = cycleOrder(a, "id", false);
    expect(b).toEqual([desc("id")]);
    const c = cycleOrder(b, "id", false);
    expect(c).toEqual([]);
  });

  it("cycleOrder: Shift'siz tıklama diğer anahtarları bırakır", () => {
    expect(cycleOrder([asc("a"), desc("b")], "c", false)).toEqual([asc("c")]);
    expect(cycleOrder([asc("a"), asc("b")], "b", false)).toEqual([desc("b")]);
  });

  it("cycleOrder: Shift ile ekler; mevcut anahtar yerinde döner ya da kalkar", () => {
    const base = [asc("a")];
    const added = cycleOrder(base, "b", true);
    expect(added).toEqual([asc("a"), asc("b")]);
    expect(base).toEqual([asc("a")]); // girdi değişmez
    expect(cycleOrder(added, "a", true)).toEqual([desc("a"), asc("b")]);
    expect(cycleOrder([desc("a"), asc("b")], "a", true)).toEqual([asc("b")]);
  });

  it("cycleOrder: en çok 8 anahtar; 9. eklenmez ama mevcutlar döner", () => {
    const eight = ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8"].map(asc);
    expect(cycleOrder(eight, "c9", true)).toEqual(eight);
    expect(cycleOrder(eight, "c3", true)[2]).toEqual(desc("c3"));
    expect(cycleOrder(eight, "c9", false)).toEqual([asc("c9")]);
  });

  it("toRequest: boş/boşluk filtre null; dolu filtre olduğu gibi", () => {
    expect(toRequest({ order: [asc("a")], filter: "" })).toEqual({ order: [asc("a")], filter: null });
    expect(toRequest({ order: [], filter: "  \n\t" })).toEqual({ order: [], filter: null });
    expect(toRequest({ order: [], filter: " id > 5 " })).toEqual({ order: [], filter: " id > 5 " });
  });

  it("toRequest: sıralama dizisi kopyalanır", () => {
    const s = { order: [asc("a")], filter: "" };
    const r = toRequest(s);
    expect(r.order).not.toBe(s.order);
  });

  it("selectErrors: filter | order | order[i].column | genel", () => {
    const r = selectErrors([
      { path: "filter", message: "f" },
      { path: "order", message: "o" },
      { path: "order[2].column", message: "oc" },
      { path: "order[1]", message: "oi" },
      { path: "object", message: "g1" },
      { path: "bilinmeyen", message: "g2" },
    ]);
    expect(r).toEqual({ filter: ["f"], order: ["o", "oc", "oi"], general: ["g1", "g2"] });
  });
});
