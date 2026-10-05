import { describe, expect, it, vi } from "vitest";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { IpcError } from "../../ipc/errors";
import { DEFAULT_CSV_OPTIONS, defaultFileName, exportErrorHint, exportSql, exportTarget, runExport, statementLabel,  type ExportDeps } from "./export";
import { USER_CANCEL_MESSAGE } from "./result-model";

const plan = (index: number, kind: StmtPlan["kind"], charOffset = 0, charLen = 5): StmtPlan =>
  ({ index, kind, danger: null, productionWrite: false, lockedReject: false, lockedReason: null, sensitive: false, charOffset, charLen }) as StmtPlan;
const err = (code: string, message = "x") => new IpcError({ code, message, detail: null, hint: null });

describe("exportTarget (uygunluk)", () => {
  it("tam bir select ve oturum gerekir", () => {
    expect((exportTarget([plan(0, "select")], true) !== null)).toBe(true);
    expect((exportTarget([plan(0, "select")], false) !== null)).toBe(false);
    expect((exportTarget([], true) !== null)).toBe(false);
    expect((exportTarget([plan(0, "write")], true) !== null)).toBe(false);
    expect((exportTarget([plan(0, "select"), plan(1, "select")], true) !== null)).toBe(false);
    expect((exportTarget([plan(0, "write"), plan(1, "select")], true) !== null)).toBe(true);
  });
  it("yalnızca ifade metnini aktarır", () => {
    expect(exportSql("update t set a=1; select 2", plan(1, "select", 18, 8))).toBe("select 2");
  });
});

describe("defaultFileName", () => {
  const d = new Date(2026, 9, 6, 9, 5);
  it("tablo ya da sorgu + tarih", () => {
    expect(defaultFileName("kullanicilar", d)).toBe("kullanicilar-20261006-0905.csv");
    expect(defaultFileName(null, d)).toBe("sorgu-20261006-0905.csv");
    expect(defaultFileName("a/b:c", d)).toBe("a_b_c-20261006-0905.csv");
  });
});

function deps(o: Partial<ExportDeps> = {}): ExportDeps {
  return {
    askOptions: vi.fn(async () => DEFAULT_CSV_OPTIONS),
    pickPath: vi.fn(async () => "C:\\x\\a.csv"),
    exportCsv: vi.fn(async () => ({ rows: 3, bytes: 10, elapsedMs: 4 })),
    onProgress: vi.fn(),
    setRunning: vi.fn(),
    ...o,
  };
}
const input = { session: "s", sql: "select 1", relation: null, stmtIndex: 0, now: new Date(2026, 0, 1) };

describe("runExport", () => {
  it("seçenekler -> kaydet -> aktar; etiket ifadeyi gösterir", async () => {
    const d = deps();
    const r = await runExport(d, input);
    expect(r).toMatchObject({ kind: "done", path: "C:\\x\\a.csv" });
    expect(d.askOptions).toHaveBeenCalledWith("İfade 1: select 1");
    expect(d.setRunning).toHaveBeenNthCalledWith(1, true);
    expect(d.setRunning).toHaveBeenLastCalledWith(false);
  });
  it("seçenek ya da kaydet diyaloğu iptal -> aborted", async () => {
    expect(await runExport(deps({ askOptions: async () => null }), input)).toEqual({ kind: "aborted" });
    const d = deps({ pickPath: async () => null });
    expect(await runExport(d, input)).toEqual({ kind: "aborted" });
    expect(d.exportCsv).not.toHaveBeenCalled();
  });
  it("aktarma sırasında iptal -> cancelled", async () => {
    const d = deps({ exportCsv: vi.fn(async () => { throw err("query", USER_CANCEL_MESSAGE); }) });
    expect(await runExport(d, input)).toEqual({ kind: "cancelled" });
    expect(d.setRunning).toHaveBeenLastCalledWith(false);
  });
  it("hatalar koda göre ipucu alır", async () => {
    for (const [code, hint] of [["session_reset", true], ["busy", true], ["locked", true], ["config", true], ["query", false], ["network", false], ["auth", false], ["tls", false], ["invalid", false]] as const) {
      const d = deps({ exportCsv: vi.fn(async () => { throw err(code); }) });
      const r = await runExport(d, input);
      expect(r.kind).toBe("error");
      if (r.kind === "error") expect(r.hint !== null, code).toBe(hint);
    }
  });
  it("exportErrorHint zaman aşımı (query) için ipucu yok", () => {
    expect(exportErrorHint({ code: "query", message: "zaman aşımı", detail: null, hint: null })).toBeNull();
  });
});

describe("sanitizeFileBase / defaultFileName", () => {
  const d = new Date(2026, 9, 6, 9, 5);
  it("ters eğik çizgi, kontrol karakteri, ayrılmış ad, sondaki nokta/boşluk, uzunluk", () => {
    expect(defaultFileName("a\x5cb", d)).toBe("a_b-20261006-0905.csv");
    expect(defaultFileName("ab", d)).toBe("a_b-20261006-0905.csv");
    for (const n of ["CON", "con", "nul", "COM1", "lpt9", "CON.txt", "aux"]) expect(defaultFileName(n, d)).toBe("_" + n + "-20261006-0905.csv");
    expect(defaultFileName("COM10", d)).toBe("COM10-20261006-0905.csv");
    expect(defaultFileName("tablo. . ", d)).toBe("tablo-20261006-0905.csv");
    expect(defaultFileName("x".repeat(300), d)).toBe("x".repeat(100) + "-20261006-0905.csv");
  });
});

describe("exportTarget seçili ifade", () => {
  const ps = [plan(0, "select"), plan(1, "select"), plan(2, "write")];
  it("açık indeks select ise o; değilse kapalı; bilinmiyorsa eski kural", () => {
    expect(exportTarget(ps, true, 1)?.index).toBe(1);
    expect(exportTarget(ps, true, 2)).toBeNull();
    expect(exportTarget(ps, true)).toBeNull();
    expect(exportTarget([plan(0, "select"), plan(1, "write")], true, 9)?.index).toBe(0);
    expect(exportTarget(ps, false, 1)).toBeNull();
    expect((exportTarget(ps, true, 0) !== null)).toBe(true);
  });
  it("statementLabel boşlukları sadeleştirir ve 80 karakterde keser", () => {
    expect(statementLabel(2, "select  1")).toBe("İfade 3: select 1");
    expect(statementLabel(0, "x".repeat(100))).toBe("İfade 1: " + "x".repeat(80) + "…");
  });
});
