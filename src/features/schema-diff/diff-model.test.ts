import { describe, expect, it } from "vitest";
import { tsd } from "../../app/strings-diff";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import {
  CATEGORIES, categoryOf, compareBlocked, counts, defaultFilters, emptySide, entryFromTarget, errorText, foldSearch,
  gutter, initialSelection, itemLabel, kindLabel, matches, problemText, progressView, sameSide, scriptBlocked,
  selectAllState, selectedKeys, selectionSummary, setMany, suggestScriptName, swapSides, tabTitle, targetBanner,
  toggleKey, visibleItems,
} from "./diff-model";

function item(o: Partial<DiffItem> = {}): DiffItem {
  return {
    key: "k", kind: "table", targetKind: null, name: "t", table: null, signature: null, status: "different",
    problem: null, risk: "safe", notes: [], errors: [], selectable: true, defaultSelected: true, ...o,
  };
}

describe("kategori", () => {
  it("problem durumdan baskındır, durumlar eşlenir", () => {
    expect(categoryOf(item({ problem: "manual", status: "same" }))).toBe("problem");
    expect(categoryOf(item({ status: "only_source" }))).toBe("onlySource");
    expect(categoryOf(item({ status: "only_target" }))).toBe("onlyTarget");
    expect(categoryOf(item({ status: "different" }))).toBe("different");
    expect(categoryOf(item({ status: "same" }))).toBe("same");
  });
  it("sayılar toplamı öğe sayısıdır; varsayılan filtre", () => {
    const items = [item({ key: "a" }), item({ key: "b", status: "same" }), item({ key: "c", problem: "load_failed" })];
    const c = counts(items);
    expect(CATEGORIES.reduce((s, k) => s + c[k], 0)).toBe(3);
    expect(defaultFilters().same).toBe(false);
    expect(defaultFilters().problem).toBe(true);
    expect(visibleItems(items, defaultFilters(), "").map((i) => i.key)).toEqual(["a", "c"]);
  });
});

describe("arama", () => {
  it("tr katlama", () => {
    expect(matches(item({ name: "İLÇE" }), foldSearch("ilçe"))).toBe(true);
    expect(matches(item({ name: "Şube" }), foldSearch("şube"))).toBe(true);
    expect(matches(item({ name: "ISIK" }), foldSearch("ısık"))).toBe(true);
    expect(matches(item({ name: "ı" }), foldSearch("I"))).toBe(true);
  });
  it("tablo, imza, tür etiketi, boş sorgu", () => {
    expect(matches(item({ name: "i", table: "siparis" }), foldSearch("sipar"))).toBe(true);
    expect(matches(item({ kind: "function", name: "f", signature: "integer, text" }), foldSearch("integer"))).toBe(true);
    expect(matches(item({ kind: "index", name: "x" }), foldSearch("indeks"))).toBe(true);
    expect(matches(item(), foldSearch("  "))).toBe(true);
    expect(matches(item({ name: "abc" }), foldSearch("zzz"))).toBe(false);
  });
});

describe("seçim", () => {
  const a = item({ key: "a" });
  const b = item({ key: "b", defaultSelected: false });
  const c = item({ key: "c", selectable: false, defaultSelected: false });
  const d = item({ key: "d" });
  const all = [a, b, c, d];
  it("initialSelection yalnız defaultSelected", () => {
    expect([...initialSelection(all)].sort()).toEqual(["a", "d"]);
  });
  it("toggleKey seçilemezde değişmez", () => {
    const s = new Set(["a"]);
    expect(toggleKey(s, c)).toBe(s);
    expect([...toggleKey(s, b)].sort()).toEqual(["a", "b"]);
    expect([...toggleKey(s, a)]).toEqual([]);
  });
  it("setMany", () => {
    expect([...setMany(new Set(), all, true)].sort()).toEqual(["a", "b", "d"]);
    expect([...setMany(new Set(["a", "b", "d", "x"]), [a, b], false)].sort()).toEqual(["d", "x"]);
  });
  it("selectAllState", () => {
    expect(selectAllState(new Set(), all)).toBe("none");
    expect(selectAllState(new Set(["a"]), all)).toBe("some");
    expect(selectAllState(new Set(["a", "b", "d"]), all)).toBe("all");
    expect(selectAllState(new Set(), [c])).toBe("none");
  });
  it("selectionSummary", () => {
    expect(selectionSummary(new Set(["a", "b", "d"]), [a, b, c])).toEqual({ selected: 3, hidden: 1 });
  });
  it("selectedKeys sonuç sırasında, olmayanı atar", () => {
    expect(selectedKeys(new Set(["d", "a", "yok"]), all)).toEqual(["a", "d"]);
  });
});

describe("seçici", () => {
  const full = { serverId: "s", databaseOid: 1, schema: "x" };
  it("sameSide", () => {
    expect(sameSide(full, { ...full })).toBe(true);
    expect(sameSide(emptySide(), emptySide())).toBe(false);
    expect(sameSide(full, { ...full, schema: null })).toBe(false);
    expect(sameSide(full, { ...full, schema: "y" })).toBe(false);
  });
  it("compareBlocked sırası", () => {
    expect(compareBlocked(full, full, true)).toBe("running");
    expect(compareBlocked(emptySide(), emptySide(), true)).toBe("running");
    expect(compareBlocked(emptySide(), full, false)).toBe("incomplete");
    expect(compareBlocked(full, full, false)).toBe("same");
    expect(compareBlocked(full, { ...full, schema: "y" }, false)).toBeNull();
  });
  it("compareBlocked: kayıtlı olmayan (silinmiş) sunucu → incomplete", () => {
    const other = { ...full, schema: "y" };
    expect(compareBlocked(full, other, false, (id) => id === "s")).toBeNull();
    expect(compareBlocked(full, other, false, () => false)).toBe("incomplete");
    expect(compareBlocked(full, { ...other, serverId: "silindi" }, false, (id) => id === "s")).toBe("incomplete");
    expect(compareBlocked({ ...full, serverId: "silindi" }, other, false, (id) => id === "s")).toBe("incomplete");
    expect(compareBlocked(full, other, true, () => false)).toBe("running");
  });
  it("scriptBlocked sırası", () => {
    const ok = { running: false, hasResult: true, stale: false, selected: 1 };
    expect(scriptBlocked(ok)).toBeNull();
    expect(scriptBlocked({ running: true, hasResult: false, stale: true, selected: 0 })).toBe("running");
    expect(scriptBlocked({ ...ok, hasResult: false, stale: true, selected: 0 })).toBe("noResult");
    expect(scriptBlocked({ ...ok, stale: true, selected: 0 })).toBe("stale");
    expect(scriptBlocked({ ...ok, selected: 0 })).toBe("noSelection");
  });
  it("swapSides", () => {
    expect(swapSides(1, 2)).toEqual([2, 1]);
  });
});

describe("entryFromTarget", () => {
  const base = { create: null, focus: null, refreshNodeId: null };
  it("şema, veritabanı, tablo", () => {
    const schema: DdlTarget = { ...base, serverId: "s", databaseOid: 7, object: { kind: "schema", oid: 3 } };
    expect(entryFromTarget(schema, "Şube")).toEqual({ serverId: "s", databaseOid: 7, schema: "Şube" });
    const db: DdlTarget = { ...base, serverId: "s", databaseOid: null, object: { kind: "database", oid: 9 } };
    expect(entryFromTarget(db, "x")).toEqual({ serverId: "s", databaseOid: 9, schema: null });
    const tbl: DdlTarget = { ...base, serverId: "s", databaseOid: 7, object: { kind: "table", oid: 5 } };
    expect(entryFromTarget(tbl, "t")).toBeNull();
  });
});

describe("adlandırma", () => {
  it("tabTitle", () => {
    expect(tabTitle({ db: "a", schema: "s" }, { db: "b", schema: "t" })).toBe("a.s ↔ b.t");
  });
  it("suggestScriptName", () => {
    const n = suggestScriptName({ db: "satış", schema: 'a"b' }, { db: "x", schema: "y/z" }, new Date(2026, 9, 31, 14, 5));
    expect(n).toBe("satış_a_b__x_y_z_20261031_1405.sql");
    const long = suggestScriptName({ db: "a".repeat(300), schema: "s" }, { db: "b", schema: "t" }, new Date(2026, 9, 31, 14, 5));
    expect(long.length).toBeLessThanOrEqual(120);
    expect(long.endsWith(".sql")).toBe(true);
  });
  it("suggestScriptName: kesimden sonra sondaki boşluk ve noktalar atılır", () => {
    // Kesim (116 karakter) "a" ile 113 karakter, ardından ". ." üçlüsüne denk gelir.
    const n = suggestScriptName({ db: `${"a".repeat(113)}. .x`, schema: "s" }, { db: "b", schema: "t" }, new Date(2026, 9, 31, 14, 5));
    expect(n).toBe(`${"a".repeat(113)}.sql`);
    const dots = suggestScriptName({ db: `${"a".repeat(114)}..`, schema: "s" }, { db: "b", schema: "t" }, new Date(2026, 9, 31, 14, 5));
    expect(dots).toBe(`${"a".repeat(114)}.sql`);
  });
});

describe("görünüm yardımcıları", () => {
  it("progressView", () => {
    expect(progressView(null)).toEqual({ percent: null, text: "" });
    expect(progressView({ loaded: 0, total: 0 }).percent).toBeNull();
    expect(progressView({ loaded: 5, total: 10 })).toEqual({ percent: 50, text: "5 / 10" });
    expect(progressView({ loaded: 12, total: 10 }).percent).toBe(100);
  });
  it("itemLabel, kindLabel, problemText", () => {
    expect(itemLabel(item({ kind: "index", table: "t", name: "i" }))).toBe("t.i");
    expect(itemLabel(item({ kind: "function", name: "f", signature: "integer, text" }))).toBe("f(integer, text)");
    expect(itemLabel(item({ name: "t" }))).toBe("t");
    expect(kindLabel("index")).toBe("İndeks");
    expect(kindLabel("schema")).toBe("schema");
    expect(problemText(item({ problem: "manual" }))).toBe("Elle gerekiyor");
    expect(problemText(item({ problem: "load_failed" }))).toBe("Yüklenemedi");
    expect(problemText(item({ problem: "build_failed" }))).toBe("Üretilemedi");
    expect(problemText(item())).toBeNull();
  });
  it("errorText: tsd.errors'taki her kod eşlenmiş metni döndürür", () => {
    const p = (code: string) => ({ code, message: "m", detail: null, hint: "ipucu" });
    const codes = Object.keys(tsd.errors);
    expect(codes).toEqual(expect.arrayContaining(["not_connected", "connection_lost", "unknown_server", "connectFailed"]));
    for (const code of codes) expect(errorText(p(code)), code).toBe(tsd.errors[code]);
  });
  it("errorText", () => {
    const p = (code: string) => ({ code, message: "m", detail: null, hint: null });
    expect(errorText(p("same_schema"))).toBe("Kaynak ve hedef aynı şema.");
    expect(errorText(p("write_failed"))).toBe("Dosya yazılamadı.");
    expect(errorText(p("diff_not_found"))).toContain("Sonuç artık yok");
    expect(errorText(p("bilinmeyen"))).toBe("m");
    expect(errorText({ ...p("bilinmeyen"), hint: "ipucu" })).toBe("m ipucu");
    expect(errorText(p("password_required"))).toBe("m Sunucu bağlantısını kesip yeniden bağlanın; parola istenir.");
  });
  it("targetBanner", () => {
    const cfg = { id: "s", name: "Sunucu A", label: "Üretim", readOnlyLock: true } as ServerConfig;
    expect(targetBanner(cfg)).toEqual({ production: true, locked: true, name: "Sunucu A" });
    expect(targetBanner({ ...cfg, label: null, readOnlyLock: false })).toEqual({ production: false, locked: false, name: "Sunucu A" });
    expect(targetBanner(null)).toEqual({ production: false, locked: false, name: null });
  });
  it("gutter", () => {
    expect(gutter({ kind: "removed", text: "", sourceNo: null, targetNo: 4 })).toEqual({ target: "4", source: "", sign: "−" });
    expect(gutter({ kind: "added", text: "", sourceNo: 2, targetNo: null })).toEqual({ target: "", source: "2", sign: "+" });
    expect(gutter({ kind: "same", text: "", sourceNo: 2, targetNo: 3 })).toEqual({ target: "3", source: "2", sign: " " });
  });
});
