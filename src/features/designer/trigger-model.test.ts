import { describe, expect, it } from "vitest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TriggerDef } from "../../ipc/bindings/TriggerDef";
import type { TriggerEvents } from "../../ipc/bindings/TriggerEvents";
import {
  asTrigger, availability, functionFromLookup, functionOptionIndex, normalizeTrigger, rebuildNote, restoreCleared, setAt, structureChanged,
  stashFor, toggleUpdateColumn, type TriggerStash,
  triggerPatch, updateColumnChoices,
} from "./trigger-model";

const ev = (p: Partial<TriggerEvents> = {}): TriggerEvents => ({
  insert: false, update: false, updateColumns: [], delete: false, truncate: false, ...p,
});
const t = (p: Partial<TriggerDef> = {}): TriggerDef => ({
  schema: "public", table: "t", name: "trg", timing: "after", events: ev({ insert: true }), level: "row", when: null,
  functionSchema: "public", functionName: "f", args: [], oldTable: null, newTable: null, constraint: false,
  deferrable: false, initiallyDeferred: false, enabled: "origin", comment: null, editable: true, unsupported: [], ...p,
});

describe("asTrigger / triggerPatch", () => {
  it("yalnız trigger daraltılır", () => {
    const d: ObjectDef = { kind: "trigger", ...t() };
    expect(asTrigger(d)?.name).toBe("trg");
    expect(asTrigger(null)).toBeNull();
    expect(asTrigger({ kind: "schema", name: "s", owner: null, comment: null })).toBeNull();
  });
  it("triggerPatch: alan güncellenir, kind korunur; başka türde aynı nesne", () => {
    const d: ObjectDef = { kind: "trigger", ...t() };
    const n = triggerPatch(d, { name: "x" });
    expect(n.kind).toBe("trigger");
    expect(asTrigger(n)?.name).toBe("x");
    const s: ObjectDef = { kind: "schema", name: "s", owner: null, comment: null };
    expect(triggerPatch(s, { name: "x" })).toBe(s);
  });
  it("triggerPatch editable/unsupported alanlarını düşürmez", () => {
    const d: ObjectDef = { kind: "trigger", ...t({ editable: false, unsupported: ["x"] }) };
    const n = asTrigger(triggerPatch(d, { comment: "c" }))!;
    expect(n.editable).toBe(false);
    expect(n.unsupported).toEqual(["x"]);
  });
});

describe("availability", () => {
  it("varsayılan AFTER INSERT ROW: hepsi açık (deferrable hariç)", () => {
    expect(availability(t())).toEqual({
      row: true, statement: true, referencing: true, oldTable: false, newTable: true, deferrable: false, updateColumns: false,
      before: true, truncate: true, constraint: true,
    });
  });
  it("TRUNCATE → yalnız STATEMENT; constraint kapalı", () => {
    const a = availability(t({ events: ev({ truncate: true }), level: "statement" }));
    expect(a.row).toBe(false);
    expect(a.statement).toBe(true);
    expect(a.constraint).toBe(false);
    expect(a.referencing).toBe(false);
  });
  it("constraint → yalnız AFTER + ROW; deferrable açık; TRUNCATE ve REFERENCING kapalı", () => {
    const a = availability(t({ constraint: true }));
    expect(a.before).toBe(false);
    expect(a.statement).toBe(false);
    expect(a.row).toBe(true);
    expect(a.deferrable).toBe(true);
    expect(a.truncate).toBe(false);
    expect(a.referencing).toBe(false);
  });
  it("REFERENCING yalnız AFTER, tek olay, UPDATE OF yok", () => {
    expect(availability(t({ timing: "before" })).referencing).toBe(false);
    expect(availability(t({ events: ev({ insert: true, delete: true }) })).referencing).toBe(false);
    expect(availability(t({ events: ev({ update: true, updateColumns: ["a"] }) })).referencing).toBe(false);
    expect(availability(t({ events: ev({ update: true }) })).referencing).toBe(true);
    expect(availability(t({ events: ev() })).referencing).toBe(false);
  });
  it("OLD TABLE yalnız UPDATE/DELETE, NEW TABLE yalnız INSERT/UPDATE", () => {
    expect(availability(t({ events: ev({ insert: true }) }))).toMatchObject({ oldTable: false, newTable: true });
    expect(availability(t({ events: ev({ update: true }) }))).toMatchObject({ oldTable: true, newTable: true });
    expect(availability(t({ events: ev({ delete: true }) }))).toMatchObject({ oldTable: true, newTable: false });
    expect(availability(t({ timing: "before", events: ev({ update: true }) }))).toMatchObject({ oldTable: false, newTable: false });
  });
  it("UPDATE OF yalnız UPDATE seçiliyken", () => {
    expect(availability(t({ events: ev({ update: true }) })).updateColumns).toBe(true);
    expect(availability(t({ events: ev({ insert: true }) })).updateColumns).toBe(false);
  });
});

describe("normalizeTrigger", () => {
  it("geçerli tanımda değişmez (aynı içerik)", () => {
    const d = t({ events: ev({ update: true, updateColumns: ["a"] }), when: "NEW.a > 0" });
    expect(normalizeTrigger(d)).toEqual(d);
  });
  it("TRUNCATE → level statement", () => {
    expect(normalizeTrigger(t({ events: ev({ truncate: true }), level: "row" })).level).toBe("statement");
  });
  it("constraint kapanınca deferrable/initiallyDeferred false", () => {
    const n = normalizeTrigger(t({ constraint: false, deferrable: true, initiallyDeferred: true }));
    expect(n.deferrable).toBe(false);
    expect(n.initiallyDeferred).toBe(false);
  });
  it("deferrable kapanınca initiallyDeferred false", () => {
    expect(normalizeTrigger(t({ constraint: true, deferrable: false, initiallyDeferred: true })).initiallyDeferred).toBe(false);
  });
  it("constraint açılınca AFTER + ROW, TRUNCATE ve geçiş tabloları temizlenir", () => {
    const n = normalizeTrigger(
      t({ constraint: true, timing: "before", level: "statement", events: ev({ insert: true, truncate: true }), newTable: "n" }),
    );
    expect(n.timing).toBe("after");
    expect(n.level).toBe("row");
    expect(n.events.truncate).toBe(false);
    expect(n.newTable).toBeNull();
  });
  it("olay sayısı artınca geçiş tabloları temizlenir", () => {
    const n = normalizeTrigger(t({ events: ev({ insert: true, update: true }), oldTable: "o", newTable: "n" }));
    expect(n.oldTable).toBeNull();
    expect(n.newTable).toBeNull();
  });
  it("UPDATE OF → geçiş tabloları temizlenir; BEFORE → temizlenir", () => {
    expect(normalizeTrigger(t({ events: ev({ update: true, updateColumns: ["a"] }), newTable: "n" })).newTable).toBeNull();
    expect(normalizeTrigger(t({ timing: "before", newTable: "n" })).newTable).toBeNull();
  });
  it("olaya uymayan tek geçiş tablosu temizlenir (INSERT → OLD TABLE yok; DELETE → NEW TABLE yok)", () => {
    const a = normalizeTrigger(t({ events: ev({ insert: true }), oldTable: "o", newTable: "n" }));
    expect(a.oldTable).toBeNull();
    expect(a.newTable).toBe("n");
    const b = normalizeTrigger(t({ events: ev({ delete: true }), oldTable: "o", newTable: "n" }));
    expect(b.oldTable).toBe("o");
    expect(b.newTable).toBeNull();
  });
  it("UPDATE kapanınca updateColumns temizlenir", () => {
    expect(normalizeTrigger(t({ events: ev({ insert: true, updateColumns: ["a"] }) })).events.updateColumns).toEqual([]);
  });
});

describe("functionFromLookup / functionOptionIndex", () => {
  it("value = ad, detail = şema (R1)", () => {
    expect(functionFromLookup({ value: "audit", detail: "app" })).toEqual({ functionSchema: "app", functionName: "audit" });
  });
  it("detail yoksa yedek şema", () => {
    expect(functionFromLookup({ value: "audit", detail: null }, "public")).toEqual({ functionSchema: "public", functionName: "audit" });
  });
  it("taslaktaki şema+ad listedeki öğeyle eşleşir; yoksa -1", () => {
    const items = [{ value: "a", detail: "s1" }, { value: "a", detail: "s2" }];
    expect(functionOptionIndex(items, t({ functionSchema: "s2", functionName: "a" }))).toBe(1);
    expect(functionOptionIndex(items, t({ functionSchema: "s3", functionName: "a" }))).toBe(-1);
  });
});

describe("structureChanged / rebuildNote", () => {
  it("ad, etkinlik, yorum yapı değişikliği değildir", () => {
    expect(structureChanged(t(), t({ name: "x", enabled: "disabled", comment: "c" }))).toBe(false);
    expect(structureChanged(t(), t({ when: "true" }))).toBe(true);
    expect(structureChanged(t(), t({ events: ev({ insert: true, updateColumns: [] }) }))).toBe(false);
    expect(structureChanged(t(), t({ args: ["1"] }))).toBe(true);
  });
  it("create ya da yapı aynıysa not yok", () => {
    expect(rebuildNote(null, t({ when: "true" }))).toBeNull();
    expect(rebuildNote(t(), t({ name: "x" }))).toBeNull();
  });
  it("sürüm bilinmiyor: PG 14+ OR REPLACE (Güvenli) ve eski DROP + CREATE (Yıkıcı) birlikte anlatılır", () => {
    const n = rebuildNote(t(), t({ level: "statement" }))!;
    expect(n).toContain("OR REPLACE");
    expect(n).toContain("DROP + CREATE");
    expect(n).toContain("Yıkıcı");
    expect(n).toContain("14");
  });
  it("sürüm biliniyorsa tek durum; constraint trigger her sürümde DROP + CREATE", () => {
    expect(rebuildNote(t(), t({ when: "true" }), 150000)).not.toContain("DROP + CREATE");
    expect(rebuildNote(t(), t({ when: "true" }), 130000)).not.toContain("OR REPLACE");
    const c = rebuildNote(t({ constraint: true }), t({ constraint: true, deferrable: true }))!;
    expect(c).toContain("DROP + CREATE");
    expect(c).not.toContain("Güvenli");
  });
});

describe("UPDATE OF çipleri / argümanlar", () => {
  it("toggleUpdateColumn: ekleme sona, kaldırma çıkarır, tekrar yok", () => {
    expect(toggleUpdateColumn(["b"], "a", true)).toEqual(["b", "a"]);
    expect(toggleUpdateColumn(["b", "a"], "a", true)).toEqual(["b", "a"]);
    expect(toggleUpdateColumn(["b", "a"], "b", false)).toEqual(["a"]);
  });
  it("updateColumnChoices: tabloda olmayan seçili kolon başta korunur", () => {
    expect(updateColumnChoices(["a", "b"], ["z", "a"])).toEqual(["z", "a", "b"]);
  });
  it("setAt: sınır dışı → aynı dizi", () => {
    const l = ["x"];
    expect(setAt(l, 0, "y")).toEqual(["y"]);
    expect(setAt(l, 3, "y")).toBe(l);
  });
});

describe("restoreCleared (kapanıp açılan alanların değeri geri gelir)", () => {
  /** Formun `patch` akışı: alanı uygula → normalize et → restoreCleared. */
  const step = (prev: TriggerDef, p: Partial<TriggerDef>, stash: TriggerStash) =>
    restoreCleared(prev, normalizeTrigger({ ...prev, ...p }), stash);

  it("UPDATE kapat/aç: UPDATE OF kolonları geri gelir, stash boşalır", () => {
    const d0 = t({ events: ev({ insert: true, update: true, updateColumns: ["a", "b"] }) });
    const s1 = step(d0, { events: { ...d0.events, update: false } }, {});
    expect(s1.def.events.updateColumns).toEqual([]);
    expect(s1.stash.updateColumns).toEqual(["a", "b"]);
    const s2 = step(s1.def, { events: { ...s1.def.events, update: true } }, s1.stash);
    expect(s2.def.events.updateColumns).toEqual(["a", "b"]);
    expect(s2.stash.updateColumns).toBeUndefined();
  });
  it("ikinci olay aç/kapa: OLD/NEW TABLE geri gelir", () => {
    const d0 = t({ events: ev({ update: true }), oldTable: "o", newTable: "n" });
    const s1 = step(d0, { events: { ...d0.events, insert: true } }, {});
    expect(s1.def.oldTable).toBeNull();
    expect(s1.def.newTable).toBeNull();
    expect(s1.stash).toEqual({ oldTable: "o", newTable: "n" });
    const s2 = step(s1.def, { events: { ...s1.def.events, insert: false } }, s1.stash);
    expect(s2.def.oldTable).toBe("o");
    expect(s2.def.newTable).toBe("n");
    expect(s2.stash).toEqual({});
  });
  it("constraint aç/kapa: NEW TABLE geri gelir", () => {
    const d0 = t({ events: ev({ insert: true }), newTable: "n" });
    const s1 = step(d0, { constraint: true }, {});
    expect(s1.def.newTable).toBeNull();
    expect(s1.stash).toEqual({ newTable: "n" });
    const s2 = step(s1.def, { constraint: false }, s1.stash);
    expect(s2.def.newTable).toBe("n");
    expect(s2.stash).toEqual({});
  });
  it("açıkken elle boşaltılan alan saklanmaz; olaya uymayan alan açılana dek bekler", () => {
    const d0 = t({ events: ev({ insert: true }), newTable: "n" });
    expect(step(d0, { newTable: null }, {}).stash).toEqual({});
    // DELETE'e geçiş: NEW TABLE kapanır, saklanır; OLD TABLE boş olduğundan geri gelmez.
    const s1 = step(d0, { events: ev({ delete: true }) }, {});
    expect(s1.def.newTable).toBeNull();
    expect(s1.stash).toEqual({ newTable: "n" });
    const s2 = step(s1.def, { events: ev({ delete: true, update: false }) }, s1.stash);
    expect(s2.stash).toEqual({ newTable: "n" });
    const s3 = step(s2.def, { events: ev({ insert: true }) }, s2.stash);
    expect(s3.def.newTable).toBe("n");
  });
  it("geri gelen tanım normalize edilmiş kalır; giriş nesneleri değişmez", () => {
    const stash: TriggerStash = { updateColumns: ["a"], newTable: "n" };
    const prev = t({ events: ev({ insert: true }) });
    const next = normalizeTrigger(t({ events: ev({ update: true }) }));
    const r = restoreCleared(prev, next, stash);
    expect(r.def.events.updateColumns).toEqual(["a"]);
    // UPDATE OF geri gelince REFERENCING kapalıdır: NEW TABLE beklemede kalır.
    expect(r.def.newTable).toBeNull();
    expect(r.stash).toEqual({ newTable: "n" });
    expect(normalizeTrigger(r.def)).toEqual(r.def);
    expect(stash).toEqual({ updateColumns: ["a"], newTable: "n" });
    expect(next.events.updateColumns).toEqual([]);
  });
});

describe("stashFor (saklananlar yalnız üretildikleri taslak sürümünde geçerli)", () => {
  it("sürüm eşleşirse saklananlar; değilse (yeniden yükleme, uygulama sonrası yükleme, başka yerden düzenleme) boş", () => {
    const s: TriggerStash = { newTable: "n" };
    expect(stashFor(s, 7, 7)).toBe(s);
    expect(stashFor(s, 7, 8)).toEqual({});
    expect(stashFor(s, null, 0)).toEqual({});
  });
});
