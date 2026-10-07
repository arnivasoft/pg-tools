import { describe, expect, it } from "vitest";
import { tdata } from "../../app/strings-data";
import type { DataOutcome } from "../../ipc/bindings/DataOutcome";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { RowChange } from "../../ipc/bindings/RowChange";
import { errorText } from "../../ipc/errors";
import { classifyDataError, fieldIssues, isConnectionCode, outcomeText } from "./data-errors";

type Failed = Extract<DataOutcome, { outcome: "failed" }>;

const p = (code: string, message = `m:${code}`, detail: string | null = null, hint: string | null = null): ErrorPayload =>
  ({ code, message, detail, hint });

const failed = (over: Partial<Failed> = {}): Failed => ({
  outcome: "failed",
  failedIndex: 0,
  sqlstate: null,
  position: null,
  error: p("query", "ileti", "PG ayrıntısı", null),
  conflict: false,
  outcomeUnknown: false,
  ...over,
});

const t = (v: string) => ({ kind: "text" as const, value: v });

describe("classifyDataError", () => {
  it("yalnızca code'a bakar", () => {
    const table: [string, string][] = [
      ["not_connected", "disconnected"],
      ["connection_lost", "disconnected"],
      ["unknown_server", "gone"],
      ["not_found", "gone"],
      ["locked", "locked"],
      ["plan_changed", "plan_changed"],
      ["confirmation_required", "confirmation_required"],
      ["production_confirmation_required", "confirmation_required"],
      ["invalid_def", "invalid_def"],
      ["query", "other"],
      ["row_conflict", "other"],
      ["unknown", "other"],
    ];
    for (const [code, cls] of table) expect(classifyDataError(p(code)), code).toBe(cls);
    expect(classifyDataError(p("query", "plan_changed"))).toBe("other");
  });

  it("isConnectionCode", () => {
    expect(["not_connected", "connection_lost", "unknown_server"].every(isConnectionCode)).toBe(true);
    expect(isConnectionCode("not_found")).toBe(false);
    expect(isConnectionCode("query")).toBe(false);
  });
});

describe("outcomeText", () => {
  const conflict = failed({ conflict: true, sqlstate: null, error: p("row_conflict", "satır değişmiş") });

  it("UPDATE/DELETE çakışması: değiştirilmiş/silinmiş metni", () => {
    const u = outcomeText(conflict, { changeKind: "update" });
    expect(u.kind).toBe("conflict");
    expect(u.text).toBe(tdata.conflict.updateDelete);
    expect(outcomeText(conflict, { changeKind: "delete" }).text).toBe(tdata.conflict.updateDelete);
    expect(outcomeText(conflict, { changeKind: null }).text).toBe(tdata.conflict.updateDelete);
  });

  it("INSERT çakışması farklı metin (düşürüldü)", () => {
    const i = outcomeText(conflict, { changeKind: "insert" });
    expect(i.kind).toBe("conflict");
    expect(i.text).toBe(tdata.conflict.insert);
    expect(i.text).not.toBe(tdata.conflict.updateDelete);
    expect(i.text).toContain("düşürüldü");
  });

  it("outcomeUnknown: belirsiz sonuç metni", () => {
    const r = outcomeText(failed({ outcomeUnknown: true, error: p("connection_lost", "koptu") }), { changeKind: "update" });
    expect(r.kind).toBe("unknown");
    expect(r.text).toBe(tdata.outcome.unknown);
    expect(r.text).toContain("yeniden yükleyin");
  });

  it("55P03: PG iletisi + satır kilidi ipucu", () => {
    const e = p("query", "kilit alınamadı", "could not obtain lock on row", null);
    const r = outcomeText(failed({ sqlstate: "55P03", error: e }), { changeKind: "update" });
    expect(r.kind).toBe("failed");
    expect(r.text).toBe(errorText(e));
    expect(r.hint).toBe(tdata.outcome.rowLocked);
  });

  it("diğer PG hatası iletisi aynen geçer, ipucu yok", () => {
    const e = p("query", "Benzersizlik ihlali", "duplicate key value violates unique constraint \"t_pkey\"", "ipucu");
    const r = outcomeText(failed({ sqlstate: "23505", error: e }), { changeKind: "insert" });
    expect(r).toEqual({ kind: "failed", text: errorText(e) });
    const plain = p("canceled", "İptal edildi");
    expect(outcomeText(failed({ error: plain }), { changeKind: null }).text).toBe("İptal edildi");
  });
});

describe("fieldIssues", () => {
  const changes: RowChange[] = [
    { op: "insert", values: [{ column: "a", value: t("x") }, { column: "b", value: { kind: "null" } }, { column: "c", value: { kind: "default" } }] },
    { op: "update", key: ["1"], version: "100", set: [{ column: "name", value: t("y") }, { column: "age", value: t("3") }] },
    { op: "delete", key: ["2"], version: "101" },
  ];
  const set = { changes };

  it("changes[i].set[j] / values[j] → hücre `${i}:${kolon}`", () => {
    const r = fieldIssues([
      { path: "changes[1].set[0]", message: "s0" },
      { path: "changes[1].set[1]", message: "s1" },
      { path: "changes[0].values[2]", message: "v2" },
      { path: "changes[0].values[2]", message: "v2b" },
    ], set);
    expect([...r.cell.entries()]).toEqual([["1:name", ["s0"]], ["1:age", ["s1"]], ["0:c", ["v2", "v2b"]]]);
    expect(r.row.size).toBe(0);
    expect(r.general).toEqual([]);
  });

  it("changes[i], key, key[j], version → satır", () => {
    const r = fieldIssues([
      { path: "changes[2]", message: "r" },
      { path: "changes[2].key", message: "k" },
      { path: "changes[2].key[0]", message: "k0" },
      { path: "changes[1].version", message: "ver" },
    ], set);
    expect(r.row.get(2)).toEqual(["r", "k", "k0"]);
    expect(r.row.get(1)).toEqual(["ver"]);
    expect(r.cell.size).toBe(0);
  });

  it("object, changes ve bilinmeyen yollar → genel", () => {
    const r = fieldIssues([
      { path: "object", message: "o" },
      { path: "changes", message: "c" },
      { path: "columns[1].key", message: "u" },
      { path: "changes[9]", message: "aralık dışı" },
    ], set);
    expect(r.general).toEqual(["o", "c", "u", "aralık dışı"]);
  });

  it("çözülemeyen hücre yolu (tür/indeks uyuşmazlığı) satıra düşer", () => {
    const r = fieldIssues([
      { path: "changes[0].set[0]", message: "insert'te set" },
      { path: "changes[1].set[5]", message: "yok" },
      { path: "changes[2].values[0]", message: "delete" },
    ], set);
    expect(r.cell.size).toBe(0);
    expect(r.row.get(0)).toEqual(["insert'te set"]);
    expect(r.row.get(1)).toEqual(["yok"]);
    expect(r.row.get(2)).toEqual(["delete"]);
  });
});
