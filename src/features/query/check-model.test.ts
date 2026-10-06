import { ChangeSet } from "@codemirror/state";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CheckDiag } from "../../ipc/bindings/CheckDiag";
import type { CheckTarget } from "../../ipc/bindings/CheckTarget";
import {
  CHECK_DEBOUNCE_MS,
  canCheck,
  checkTarget,
  createCheckScheduler,
  diagsToMarks,
  mapMarks,
  type CheckGate,
  type CheckView,
  type LintMark,
} from "./check-model";

const open: CheckGate = { enabled: true, session: "S", status: "idle", op: null, tx: "idle", serverGone: false };

const diag = (p: Partial<CheckDiag>): CheckDiag => ({
  stmtIndex: 0,
  charOffset: 0,
  charLen: 0,
  position: null,
  message: "m",
  sqlstate: "42601",
  severity: "error",
  ...p,
});

describe("canCheck", () => {
  it("yalnızca açık + boşta + Failed dışı + etkin", () => {
    expect(canCheck(open)).toBe(true);
    expect(canCheck({ ...open, tx: "inTransaction" })).toBe(true);
    for (const g of [
      null,
      { ...open, enabled: false },
      { ...open, session: null },
      { ...open, status: "noSession" as const },
      { ...open, status: "opening" as const },
      { ...open, status: "running" as const },
      { ...open, status: "cancelling" as const },
      { ...open, status: "lost" as const },
      { ...open, op: "export" as const },
      { ...open, op: "more" as const },
      { ...open, op: "tx" as const },
      { ...open, tx: "failed" as const },
      { ...open, serverGone: true },
    ]) {
      expect(canCheck(g), JSON.stringify(g)).toBe(false);
    }
  });
});

describe("checkTarget / diagsToMarks (UTF-16 ↔ scalar)", () => {
  const text = "select '😀';\nselect nope from t;";
  it("hedef scalar ofsetlerle", () => {
    const at = text.indexOf("nope");
    expect(checkTarget({ text, cursor: at, visibleFrom: 0, visibleTo: text.length })).toEqual({
      cursor: at - 1,
      visibleStart: 0,
      visibleEnd: text.length - 1,
    });
  });
  it("konumlu tanı: emoji sonrasında doğru kelime", () => {
    const stmtScalar = [..."select '😀';\n"].length;
    const d = diag({ stmtIndex: 1, charOffset: stmtScalar, charLen: [..."select nope from t"].length, position: 8 });
    const [m] = diagsToMarks(text, [d]);
    expect(text.slice(m!.from, m!.to)).toBe("nope");
    expect(text.slice(m!.stmtFrom, m!.stmtTo)).toBe("select nope from t");
  });
  it("konumsuz tanı bütün ifadeyi işaretler; bilgi düzeyi korunur", () => {
    const [m] = diagsToMarks(text, [diag({ charOffset: 0, charLen: 10, severity: "info", sqlstate: null })]);
    expect(text.slice(m!.from, m!.to)).toBe("select '😀'");
    expect(m!.severity).toBe("info");
    expect(m!.sqlstate).toBeNull();
  });
});

describe("mapMarks", () => {
  const text = "select 1;\nselect nope;";
  const marks: LintMark[] = diagsToMarks(text, [diag({ charOffset: 10, charLen: 11, position: 8 })]);
  it("ifadesine dokunan düzenleme işareti kaldırır", () => {
    const cs = ChangeSet.of({ from: text.indexOf("nope") + 2, insert: "x" }, text.length);
    expect(mapMarks(marks, cs)).toEqual([]);
    const atEnd = ChangeSet.of({ from: text.length - 1, insert: "x" }, text.length);
    expect(mapMarks(marks, atEnd)).toEqual([]);
  });
  it("başka ifadedeki düzenleme işareti kaydırır", () => {
    const cs = ChangeSet.of({ from: 7, to: 8, insert: "123" }, text.length);
    const out = mapMarks(marks, cs);
    expect(out).toHaveLength(1);
    const next = "select 123;\nselect nope;";
    expect(next.slice(out[0]!.from, out[0]!.to)).toBe("nope");
    expect(next.slice(out[0]!.stmtFrom, out[0]!.stmtTo)).toBe("select nope");
  });
});

describe("createCheckScheduler", () => {
  let gate: CheckGate | null;
  let view: CheckView;
  let calls: { session: string; sql: string; target: CheckTarget; resolve: (d: CheckDiag[]) => void; reject: (e: unknown) => void }[];
  let applied: LintMark[][];
  const make = () =>
    createCheckScheduler({
      gate: () => gate,
      view: () => view,
      request: (session, sql, target) =>
        new Promise((resolve, reject) => {
          calls.push({ session, sql, target, resolve, reject });
        }),
      apply: (m) => applied.push(m),
    });
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    gate = { ...open };
    view = { text: "select nope;", cursor: 3, visibleFrom: 0, visibleTo: 12 };
    calls = [];
    applied = [];
  });
  afterEach(() => vi.useRealTimers());

  it("debounce: 600 ms durunca tek istek", () => {
    const s = make();
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS - 1);
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS - 1);
    expect(calls).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(calls).toHaveLength(1);
    expect(calls[0]!.session).toBe("S");
    expect(calls[0]!.target).toEqual({ cursor: 3, visibleStart: 0, visibleEnd: 12 });
  });

  it("eski sonuç (arada düzenleme) yok sayılır; güncel sonuç uygulanır", async () => {
    const s = make();
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    s.edited();
    calls[0]!.resolve([diag({ charLen: 11, position: 8 })]);
    await flush();
    expect(applied).toEqual([]);
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    calls[1]!.resolve([diag({ charLen: 11, position: 8 })]);
    await flush();
    expect(applied).toHaveLength(1);
    expect(view.text.slice(applied[0]![0]!.from, applied[0]![0]!.to)).toBe("nope");
  });

  it("çalıştırma öncesi invalidate süren denetimin sonucunu düşürür", async () => {
    const s = make();
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    s.invalidate();
    calls[0]!.resolve([diag({ charLen: 11 })]);
    await flush();
    expect(applied).toEqual([]);
  });

  it("invalidate bekleyen zamanlayıcıyı iptal eder", () => {
    const s = make();
    s.edited();
    s.invalidate();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS * 2);
    expect(calls).toHaveLength(0);
  });

  it("meşgul / Failed / oturumsuz / kapalıyken istek yok", () => {
    const s = make();
    for (const g of [
      { ...open, status: "running" as const },
      { ...open, op: "export" as const },
      { ...open, tx: "failed" as const },
      { ...open, session: null, status: "noSession" as const },
      { ...open, enabled: false },
      null,
    ]) {
      gate = g;
      s.edited();
      vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    }
    expect(calls).toHaveLength(0);
  });

  it("aynı ifadede imleç hareketi yeniden istemez; başka ifadeye geçince ister", () => {
    view = { text: "select 1;\nselect 2;", cursor: 2, visibleFrom: 0, visibleTo: 19 };
    const s = make();
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    expect(calls).toHaveLength(1);
    view = { ...view, cursor: 5 };
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    expect(calls).toHaveLength(1);
    view = { ...view, cursor: 14 };
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    expect(calls).toHaveLength(2);
    // kaydırma: görünür alan değişti
    view = { ...view, visibleFrom: 10 };
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    expect(calls).toHaveLength(3);
  });

  it("sonuçlar sırasız gelirse eski istek yeniyi ezmez", async () => {
    view = { text: "select 1;\nselect 2;", cursor: 2, visibleFrom: 0, visibleTo: 19 };
    const s = make();
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    view = { ...view, cursor: 14 };
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    calls[1]!.resolve([]);
    await flush();
    calls[0]!.resolve([diag({ charLen: 8 })]);
    await flush();
    expect(applied).toEqual([[]]);
  });

  it("hata sessizdir ve aynı hedef yeniden denenebilir", async () => {
    const s = make();
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    calls[0]!.reject(new Error("x"));
    await flush();
    s.moved();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    expect(calls).toHaveLength(2);
    expect(applied).toEqual([]);
  });

  it("dispose sonrası istek ve uygulama yok", async () => {
    const s = make();
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    s.dispose();
    calls[0]!.resolve([diag({})]);
    s.edited();
    vi.advanceTimersByTime(CHECK_DEBOUNCE_MS);
    await flush();
    expect(calls).toHaveLength(1);
    expect(applied).toEqual([]);
  });
});
