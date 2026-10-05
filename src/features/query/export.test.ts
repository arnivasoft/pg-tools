import { describe, expect, it, vi } from "vitest";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { IpcError } from "../../ipc/errors";
import { DEFAULT_CSV_OPTIONS, canExport, defaultFileName, exportErrorHint, exportSql, runExport, type ExportDeps } from "./export";
import { USER_CANCEL_MESSAGE } from "./result-model";

const plan = (index: number, kind: StmtPlan["kind"], charOffset = 0, charLen = 5): StmtPlan =>
  ({ index, kind, danger: null, productionWrite: false, lockedReject: false, lockedReason: null, sensitive: false, charOffset, charLen }) as StmtPlan;
const err = (code: string, message = "x") => new IpcError({ code, message, detail: null, hint: null });

describe("canExport", () => {
  it("tam bir select ve oturum gerekir", () => {
    expect(canExport([plan(0, "select")], true)).toBe(true);
    expect(canExport([plan(0, "select")], false)).toBe(false);
    expect(canExport([], true)).toBe(false);
    expect(canExport([plan(0, "write")], true)).toBe(false);
    expect(canExport([plan(0, "select"), plan(1, "select")], true)).toBe(false);
    expect(canExport([plan(0, "write"), plan(1, "select")], true)).toBe(true);
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
    exists: vi.fn(async () => false),
    confirmOverwrite: vi.fn(async () => true),
    exportCsv: vi.fn(async () => ({ rows: 3, bytes: 10, elapsedMs: 4 })),
    onProgress: vi.fn(),
    setRunning: vi.fn(),
    ...o,
  };
}
const input = { session: "s", sql: "select 1", relation: null, now: new Date(2026, 0, 1) };

describe("runExport", () => {
  it("hedef yoksa onaysız aktarır", async () => {
    const d = deps();
    const r = await runExport(d, input);
    expect(r).toMatchObject({ kind: "done", path: "C:\\x\\a.csv" });
    expect(d.confirmOverwrite).not.toHaveBeenCalled();
    expect(d.setRunning).toHaveBeenNthCalledWith(1, true);
    expect(d.setRunning).toHaveBeenLastCalledWith(false);
  });
  it("hedef varsa onay ister; vazgeçilirse aktarmaz", async () => {
    const d = deps({ exists: vi.fn(async () => true), confirmOverwrite: vi.fn(async () => false) });
    expect(await runExport(d, input)).toEqual({ kind: "aborted" });
    expect(d.exportCsv).not.toHaveBeenCalled();
  });
  it("hedef varsa onaylanınca aktarır", async () => {
    const d = deps({ exists: vi.fn(async () => true) });
    expect((await runExport(d, input)).kind).toBe("done");
    expect(d.exportCsv).toHaveBeenCalled();
  });
  it("seçenek ya da kaydet diyaloğu iptal -> aborted", async () => {
    expect(await runExport(deps({ askOptions: async () => null }), input)).toEqual({ kind: "aborted" });
    const d = deps({ pickPath: async () => null });
    expect(await runExport(d, input)).toEqual({ kind: "aborted" });
    expect(d.exists).not.toHaveBeenCalled();
  });
  it("aktarma sırasında iptal -> cancelled", async () => {
    const d = deps({ exportCsv: vi.fn(async () => { throw err("query", USER_CANCEL_MESSAGE); }) });
    expect(await runExport(d, input)).toEqual({ kind: "cancelled" });
    expect(d.setRunning).toHaveBeenLastCalledWith(false);
  });
  it("hatalar koda göre ipucu alır", async () => {
    for (const [code, hint] of [["session_reset", true], ["busy", true], ["locked", true], ["query", false], ["config", true]] as const) {
      const d = deps({ exportCsv: vi.fn(async () => { throw err(code); }) });
      const r = await runExport(d, input);
      expect(r.kind).toBe("error");
      if (r.kind === "error") expect(r.hint !== null).toBe(hint);
    }
  });
  it("exists hatası error döner, aktarma başlamaz", async () => {
    const d = deps({ exists: vi.fn(async () => { throw err("invalid"); }) });
    expect((await runExport(d, input)).kind).toBe("error");
    expect(d.setRunning).not.toHaveBeenCalled();
  });
  it("exportErrorHint zaman aşımı (query) için ipucu yok", () => {
    expect(exportErrorHint({ code: "query", message: "zaman aşımı", detail: null, hint: null })).toBeNull();
  });
});
