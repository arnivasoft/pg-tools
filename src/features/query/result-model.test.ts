import { describe, expect, it } from "vitest";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { NoticeInfo } from "../../ipc/bindings/NoticeInfo";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import {
  MAX_TAB_ROWS,
  USER_CANCEL_MESSAGE,
  applyEvent,
  canLoadMore,
  closeMore,
  emptyResult,
  isUserCancel,
  totalRows,
  type ResultState,
} from "./result-model";

const col = [{ name: "a", typeName: "int4", typeOid: 23 }];
const started = (i: number, kind: "select" | "write" = "select"): QueryEvent => ({
  type: "started",
  stmtIndex: i,
  kind,
  columns: kind === "select" ? col : [],
  charOffset: i * 10,
});
const rowsOf = (n: number, from = 0) => Array.from({ length: n }, (_, k) => [String(from + k)]);
const rows = (i: number, n: number, more = false): QueryEvent => ({ type: "rows", stmtIndex: i, rows: rowsOf(n), more });
const notice = (message: string): NoticeInfo => ({ severity: "NOTICE", code: "00000", message, detail: null, hint: null });
const done = (i: number, notices: NoticeInfo[] = [], truncated = false): QueryEvent => ({
  type: "done",
  stmtIndex: i,
  tag: "SELECT 1",
  rowsAffected: null,
  rowsSent: 1,
  truncated,
  elapsedMs: 3,
  notices,
});
const err = (message: string, code = "query"): ErrorPayload => ({ code, message, detail: null, hint: null });
const failed = (i: number, error: ErrorPayload, notices: NoticeInfo[] = []): QueryEvent => ({
  type: "failed",
  stmtIndex: i,
  error,
  position: 5,
  charOffset: i * 10,
  notices,
});
const scriptDone = (stmtsRun: number, stoppedOnError = false): QueryEvent => ({ type: "scriptDone", stmtsRun, stoppedOnError });

const run = (evs: QueryEvent[], runId = 1, init: ResultState = emptyResult(runId)) =>
  evs.reduce((s, e) => applyEvent(s, runId, e), init);

describe("applyEvent", () => {
  it("tek ifade", () => {
    const s = run([started(0), rows(0, 3), done(0), scriptDone(1)]);
    expect(s.done).toBe(true);
    expect(s.stoppedOnError).toBe(false);
    expect(s.stmts).toHaveLength(1);
    const st = s.stmts[0];
    expect(st.status).toBe("ok");
    expect(st.rows).toEqual(rowsOf(3));
    expect(st.tag).toBe("SELECT 1");
    expect(st.elapsedMs).toBe(3);
    expect(st.columns).toEqual(col);
    expect(s.messages).toEqual([{ level: "info", stmtIndex: 0, tag: "SELECT 1", rowsAffected: null, elapsedMs: 3 }]);
  });

  it("invoke çözülmesi değil scriptDone bitirir", () => {
    const s = run([started(0), rows(0, 1), done(0)]);
    expect(s.done).toBe(false);
  });

  it("eski runId olayları yok sayılır (aynı nesne döner)", () => {
    const s0 = run([started(0)], 2);
    expect(applyEvent(s0, 1, rows(0, 5))).toBe(s0);
    expect(applyEvent(s0, 1, scriptDone(1))).toBe(s0);
    expect(applyEvent(s0, 3, failed(0, err("x")))).toBe(s0);
  });

  it("betik hata ile durur", () => {
    const s = run([started(0), done(0), started(1, "write"), failed(1, err("syntax error")), scriptDone(2, true)]);
    expect(s.done).toBe(true);
    expect(s.stoppedOnError).toBe(true);
    expect(s.stmts.map((x) => x.status)).toEqual(["ok", "failed"]);
    expect(s.stmts[1].error).toEqual({ ...err("syntax error"), position: 5, charOffset: 10 });
    expect(s.messages.at(-1)).toEqual({ level: "error", stmtIndex: 1, error: s.stmts[1].error });
  });

  it("ifade ortasında iptal → cancelled", () => {
    const s = run([started(0), rows(0, 2, true), failed(0, err(USER_CANCEL_MESSAGE)), scriptDone(1, true)]);
    expect(s.stmts[0].status).toBe("cancelled");
    expect(s.stmts[0].more).toBe(false);
    expect(s.messages.at(-1)?.level).toBe("cancelled");
  });

  it("ifadeler arası iptal: sıradaki indeks için failed (started yok) + scriptDone", () => {
    const s = run([started(0), done(0), failed(1, err(USER_CANCEL_MESSAGE)), scriptDone(1, true)]);
    expect(s.stmts).toHaveLength(2);
    expect(s.stmts[1]).toMatchObject({ index: 1, status: "cancelled", kind: "other", columns: [], rows: [], charOffset: 10 });
    expect(s.done).toBe(true);
    expect(s.stoppedOnError).toBe(true);
  });

  it("statement_timeout iptal sayılmaz", () => {
    expect(isUserCancel(err("Sorgu zaman aşımına uğradı."))).toBe(false);
    const s = run([started(0), failed(0, err("Sorgu zaman aşımına uğradı."))]);
    expect(s.stmts[0].status).toBe("failed");
  });

  it("NOTICE'lar done ve failed'da iletilere eklenir", () => {
    const s = run([
      started(0),
      done(0, [notice("n1")]),
      started(1),
      failed(1, err("boom"), [notice("n2"), notice("n3")]),
      scriptDone(2, true),
    ]);
    expect(s.messages.map((m) => m.level)).toEqual(["notice", "info", "notice", "notice", "error"]);
    expect(s.messages.filter((m) => m.level === "notice").map((m) => (m.level === "notice" ? m.notice.message : "")))
      .toEqual(["n1", "n2", "n3"]);
  });

  it("bilinmeyen ifadeye rows/done yok sayılır", () => {
    const s0 = run([started(0)]);
    expect(applyEvent(s0, 1, rows(4, 1))).toBe(s0);
    expect(applyEvent(s0, 1, done(4))).toBe(s0);
  });

  it("scriptDone sonrası qt_more rows olayları eklenir, diğerleri yok sayılır", () => {
    const s = run([started(0), rows(0, 2, true), done(0), scriptDone(1)]);
    const s2 = applyEvent(s, 1, { type: "rows", stmtIndex: 0, rows: rowsOf(2, 2), more: false });
    expect(s2.stmts[0].rows).toEqual(rowsOf(4));
    expect(s2.stmts[0].more).toBe(false);
    expect(applyEvent(s2, 1, started(1))).toBe(s2);
  });

  it("truncated aktarılır", () => {
    const s = run([started(0), rows(0, 1), done(0, [], true)]);
    expect(s.stmts[0].truncated).toBe(true);
  });
});

describe("50.000 satır sınırı", () => {
  it("aşılınca en eski ifadeler boşaltılır, dropped", () => {
    const half = MAX_TAB_ROWS / 2;
    let s = run([started(0), rows(0, half), done(0), started(1), rows(1, half), done(1)]);
    expect(totalRows(s)).toBe(MAX_TAB_ROWS);
    expect(s.stmts.some((x) => x.dropped)).toBe(false);
    s = run([started(2), rows(2, 10)], 1, s);
    expect(s.stmts[0]).toMatchObject({ dropped: true, rows: [] });
    expect(s.stmts[1].dropped).toBe(false);
    expect(s.stmts[2].rows).toHaveLength(10);
    expect(totalRows(s)).toBe(half + 10);
  });

  it("yalnızca gerektiği kadar ifade boşaltılır; satır alan ifade korunur", () => {
    let s = run([started(0), rows(0, 10), done(0), started(1), rows(1, 10), done(1), started(2)]);
    s = applyEvent(s, 1, rows(2, MAX_TAB_ROWS - 15));
    expect(s.stmts.map((x) => x.dropped)).toEqual([true, false, false]);
    s = applyEvent(s, 1, rows(2, 20));
    expect(s.stmts.map((x) => x.dropped)).toEqual([true, true, false]);
    expect(s.stmts[2].rows).toHaveLength(MAX_TAB_ROWS + 5);
  });
});

describe("canLoadMore", () => {
  const base = run([started(0), rows(0, 2), done(0), started(1), rows(1, 2, true), done(1), scriptDone(2)]);

  it("yalnızca son ifade, more ve scriptDone sonrası", () => {
    expect(canLoadMore(base, 1)).toBe(true);
    expect(canLoadMore(base, 0)).toBe(false);
  });

  it("scriptDone gelmeden false", () => {
    expect(canLoadMore(run([started(0), rows(0, 2, true), done(0)]), 0)).toBe(false);
  });

  it("son ifade satır döndürmeyen bir ifadeyse önceki ızgara kapalıdır", () => {
    const s = run([started(0), rows(0, 2, true), done(0), started(1, "write"), done(1), scriptDone(2)]);
    expect(canLoadMore(s, 0)).toBe(false);
  });

  it("closeMore (optionsChanged) sonrası false", () => {
    expect(canLoadMore(closeMore(base), 1)).toBe(false);
  });

  it("more=false ya da hata/iptal sonrası false", () => {
    expect(canLoadMore(run([started(0), rows(0, 2), done(0), scriptDone(1)]), 0)).toBe(false);
    const c = run([started(0), rows(0, 2, true), failed(0, err(USER_CANCEL_MESSAGE)), scriptDone(1, true)]);
    expect(canLoadMore(c, 0)).toBe(false);
  });

  it("dropped ya da sınırda satır varsa false", () => {
    const s = run([started(0), rows(0, MAX_TAB_ROWS, true), done(0), scriptDone(1)]);
    expect(canLoadMore(s, 0)).toBe(false);
  });
});
