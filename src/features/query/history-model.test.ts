import { describe, expect, it } from "vitest";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { filterHistory, historyEntriesFor, insertAt, searchQuery } from "./history-model";
import { emptyResult, type ResultState, type StmtResult } from "./result-model";

const text = "select 1; create role x password 'p'; select 2; select 3";
const plan = (index: number, charOffset: number, charLen: number, sensitive = false): StmtPlan =>
  ({ index, kind: "select", danger: null, productionWrite: false, lockedReject: false, lockedReason: null, sensitive, charOffset, charLen }) as StmtPlan;
const plans = [plan(0, 0, 8), plan(1, 10, 27, true), plan(2, 38, 8), plan(3, 48, 8)];
const stmt = (index: number, o: Partial<StmtResult>): StmtResult =>
  ({ index, kind: "select", columns: [], rows: [], more: false, truncated: false, charOffset: 0, status: "ok", dropped: false, ...o }) as StmtResult;
const ctx = () => {
  let n = 0;
  return { text, serverId: "s", serverName: "Srv", database: "db", atMs: 5, newId: () => `id${n++}` };
};
const result = (stmts: StmtResult[]): ResultState => ({ ...emptyResult(1), stmts, done: true });

describe("historyEntriesFor", () => {
  it("hassas ifadeyi hariç tutar, başarısızı ok:false yazar, çalışmayanı atlar", () => {
    // 4. ifade (index 3) hiç çalışmadı: sonuçta yok.
    const r = result([stmt(0, { elapsedMs: 12 }), stmt(1, { elapsedMs: 3 }), stmt(2, { status: "failed" })]);
    const out = historyEntriesFor(plans, r, ctx());
    expect(out.map((e) => e.sql)).toEqual(["select 1", "select 2"]);
    expect(out[0]).toMatchObject({ ok: true, elapsedMs: 12, serverId: "s", serverName: "Srv", database: "db", atMs: 5 });
    expect(out[1]).toMatchObject({ ok: false, elapsedMs: 0 });
    expect(new Set(out.map((e) => e.id)).size).toBe(2);
  });
  it("iptal ok:false; running atlanır; planı olmayan atlanır", () => {
    const r = result([stmt(0, { status: "cancelled" }), stmt(2, { status: "running" }), stmt(9, {})]);
    const out = historyEntriesFor(plans, r, ctx());
    expect(out).toHaveLength(1);
    expect(out[0].ok).toBe(false);
  });
  it("emoji sonrası ofsetleri karakter olarak çözer", () => {
    const t2 = "select '😀'; select 2";
    const ps = [plan(0, 0, 10), plan(1, 12, 8)];
    const out = historyEntriesFor(ps, result([stmt(1, {})]), { ...ctx(), text: t2 });
    expect(out[0].sql).toBe("select 2");
  });
});

describe("arama ve ekleme", () => {
  const e = (sql: string) => ({ id: sql, sql, serverId: "s", serverName: "", database: null, atMs: 0, elapsedMs: 0, ok: true });
  it("searchQuery boşluğu null yapar", () => {
    expect(searchQuery("  ")).toBeNull();
    expect(searchQuery(" ab ")).toBe("ab");
  });
  it("filterHistory büyük/küçük harf duyarsız", () => {
    expect(filterHistory([e("SELECT a"), e("update b")], "select").map((x) => x.sql)).toEqual(["SELECT a"]);
    expect(filterHistory([e("x")], "")).toHaveLength(1);
  });
  it("insertAt: boş belge, satır başı, satır ortası, seçimi değiştirme", () => {
    expect(insertAt("", 0, 0, "select 1")).toEqual({ doc: "select 1", cursor: 8 });
    expect(insertAt("a\n", 2, 2, "b")).toEqual({ doc: "a\nb", cursor: 3 });
    expect(insertAt("abc", 3, 3, "b")).toEqual({ doc: "abc\nb", cursor: 5 });
    expect(insertAt("a\nXYZ\nc", 2, 5, "q")).toEqual({ doc: "a\nq\nc", cursor: 3 });
    expect(insertAt("ab", 99, 99, "c").doc).toBe("ab\nc");
  });
});
