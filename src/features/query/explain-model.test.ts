import { describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ExplainResult } from "../../ipc/bindings/ExplainResult";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import {
  DEFAULT_EXPLAIN,
  confirmItems,
  effectiveOptions,
  explainWithGuards,
  optionGates,
  shownOptions,
  versionNumFromShort,
  type ExplainDeps,
} from "./explain-model";
import { tplan } from "./strings-plan";

const err = (code: string, message = code): ErrorPayload => ({ code, message, detail: null, hint: null }) as ErrorPayload;
const result = (json = "[]"): ExplainResult => ({ json, notices: [], elapsedMs: 3, rolledBack: false, sql: "EXPLAIN (FORMAT JSON) select 1", note: null });
const plan = (p: Partial<StmtPlan> = {}): StmtPlan =>
  ({ index: 0, charOffset: 0, charLen: 13, kind: "write", danger: null, productionWrite: false, lockedReject: false, lockedReason: null, sensitive: false, ...p }) as StmtPlan;

describe("versionNumFromShort", () => {
  it("kısa sürüm metnini sayıya çevirir", () => {
    expect(versionNumFromShort("16.4")).toBe(160004);
    expect(versionNumFromShort("10.23")).toBe(100023);
    expect(versionNumFromShort("17beta1")).toBe(170000);
    expect(versionNumFromShort("9.6")).toBe(90600);
    expect(versionNumFromShort(null)).toBeNull();
    expect(versionNumFromShort("x")).toBeNull();
  });
});

describe("seçenek kapıları", () => {
  it("PG 11: SETTINGS ve WAL sürüm altı, ANALYZE'sız BUFFERS kapalı; gerekçe metni var", () => {
    const g = optionGates(DEFAULT_EXPLAIN, 110_000);
    expect(g.settings.reason).toBe(tplan.gate.pg12);
    expect(g.wal.reason).toBe(tplan.gate.pg13);
    expect(g.buffers.reason).toBe(tplan.gate.buffersPre13);
    expect(g.timing.reason).toBe(tplan.gate.needsAnalyze);
    expect(g.rollback.reason).toBe(tplan.gate.needsAnalyze);
    expect(g.analyze.reason).toBeNull();
    expect(g.verbose.reason).toBeNull();
    expect(g.costs.reason).toBeNull();
  });
  it("PG 11 + ANALYZE: BUFFERS/TIMING/geri alma açılır, WAL yine sürüm altı", () => {
    const g = optionGates({ ...DEFAULT_EXPLAIN, analyze: true }, 110_000);
    expect(g.buffers.reason).toBeNull();
    expect(g.timing.reason).toBeNull();
    expect(g.rollback.reason).toBeNull();
    expect(g.wal.reason).toBe(tplan.gate.pg13);
  });
  it("PG 13+: ANALYZE'sız BUFFERS serbest, WAL ANALYZE ister", () => {
    const g = optionGates(DEFAULT_EXPLAIN, 130_000);
    expect(g.buffers.reason).toBeNull();
    expect(g.settings.reason).toBeNull();
    expect(g.wal.reason).toBe(tplan.gate.needsAnalyze);
    expect(optionGates({ ...DEFAULT_EXPLAIN, analyze: true }, 130_000).wal.reason).toBeNull();
  });
  it("sürüm bilinmiyorsa yalnız ANALYZE bağımlılıkları uygulanır", () => {
    const g = optionGates(DEFAULT_EXPLAIN, null);
    expect(g.settings.reason).toBeNull();
    expect(g.buffers.reason).toBeNull();
    expect(g.wal.reason).toBe(tplan.gate.needsAnalyze);
  });
  it("gönderilen seçenekler: istek türü ANALYZE'ı belirler, kapalı seçenekler varsayılana döner", () => {
    const chosen = { ...DEFAULT_EXPLAIN, analyze: true, wal: true, buffers: true, settings: true, timing: false, rollback: false };
    // F7: ANALYZE'sız → WAL kapanır, TIMING/geri alma varsayılan; PG 12'de ANALYZE'sız BUFFERS kapanır.
    expect(effectiveOptions(chosen, "plain", 120_000)).toEqual({ ...DEFAULT_EXPLAIN, settings: true });
    // Shift+F7: menüde ANALYZE kapalı olsa da ANALYZE.
    expect(effectiveOptions({ ...chosen, analyze: false }, "analyze", 130_000)).toEqual(chosen);
    // Düğme: menüdeki seçim; PG 11'de SETTINGS/WAL atılır.
    expect(effectiveOptions(chosen, "menu", 110_000)).toEqual({ ...chosen, wal: false, settings: false });
  });

  it("menü işaretleri: devre dışı seçenek Shift+F7'nin göndereceği değeri gösterir", () => {
    // ANALYZE kapalı: TIMING ve geri alma (varsayılan açık) işaretli görünür, WAL saklı değeriyle.
    expect(shownOptions(DEFAULT_EXPLAIN, 170_000)).toEqual(DEFAULT_EXPLAIN);
    const off = { ...DEFAULT_EXPLAIN, timing: false, rollback: false, wal: true };
    expect(shownOptions(off, 170_000)).toEqual(off);
    // Sürüm altı: PG 11'de SETTINGS/WAL hiç gönderilmez → işaretsiz; saklı değer değişmez.
    const pg11 = { ...DEFAULT_EXPLAIN, analyze: true, settings: true, wal: true };
    expect(shownOptions(pg11, 110_000)).toEqual({ ...pg11, settings: false, wal: false });
    // Açık seçenekler saklı değeri gösterir (ANALYZE dahil).
    expect(shownOptions({ ...DEFAULT_EXPLAIN, verbose: true }, null)).toEqual({ ...DEFAULT_EXPLAIN, verbose: true });
  });
});

function deps(over: Partial<ExplainDeps> = {}): ExplainDeps & { explain: ReturnType<typeof vi.fn>; confirm: ReturnType<typeof vi.fn> } {
  return {
    session: "S",
    options: DEFAULT_EXPLAIN,
    explain: vi.fn(async () => result()),
    analyze: vi.fn(async () => [plan({ danger: "deleteWithoutWhere" })]),
    confirm: vi.fn(async () => true),
    production: false,
    payload: (e: unknown) => e as ErrorPayload,
    ...over,
  } as never;
}
const snap = { text: "delete from t", scope: { type: "all" } as never };

describe("explainWithGuards", () => {
  it("onay gerekmiyorsa tek istek, onaysız", async () => {
    const d = deps();
    const out = await explainWithGuards(snap, d);
    expect(out.kind).toBe("done");
    expect(d.explain).toHaveBeenCalledTimes(1);
    expect(d.explain.mock.calls[0][4]).toBe(false);
    expect(d.confirm).not.toHaveBeenCalled();
  });
  it("confirmation_required → onay diyaloğu → confirmed: true ile yeniden dener", async () => {
    const explain = vi.fn().mockRejectedValueOnce(err("confirmation_required")).mockResolvedValueOnce(result());
    const d = deps({ explain });
    const out = await explainWithGuards(snap, d);
    expect(out.kind).toBe("done");
    expect(d.confirm).toHaveBeenCalledTimes(1);
    expect(d.confirm.mock.calls[0][0][0]).toMatchObject({ danger: "deleteWithoutWhere", text: "delete from t" });
    expect(explain.mock.calls.map((c) => c[4])).toEqual([false, true]);
  });
  it("onay reddedilirse yeniden denemez", async () => {
    const explain = vi.fn().mockRejectedValue(err("confirmation_required"));
    const d = deps({ explain, confirm: vi.fn(async () => false) });
    expect(await explainWithGuards(snap, d)).toEqual({ kind: "cancelled" });
    expect(explain).toHaveBeenCalledTimes(1);
  });
  it("onaylı istek yine onay isterse hata döner (sonsuz döngü yok)", async () => {
    const explain = vi.fn().mockRejectedValue(err("confirmation_required"));
    const out = await explainWithGuards(snap, deps({ explain }));
    expect(out).toMatchObject({ kind: "rejected", error: { code: "confirmation_required" } });
    expect(explain).toHaveBeenCalledTimes(2);
  });
  it("kilit ve diğer hatalar onay sormadan rejected", async () => {
    for (const code of ["locked", "busy", "explain_scope", "query"]) {
      const d = deps({ explain: vi.fn().mockRejectedValue(err(code)) });
      expect(await explainWithGuards(snap, d)).toMatchObject({ kind: "rejected", error: { code } });
      expect(d.confirm).not.toHaveBeenCalled();
    }
  });
  it("sınıflandırma başarısızsa rejected", async () => {
    const d = deps({ explain: vi.fn().mockRejectedValue(err("confirmation_required")), analyze: vi.fn().mockRejectedValue(err("invalid")) });
    expect(await explainWithGuards(snap, d)).toMatchObject({ kind: "rejected", error: { code: "invalid" } });
  });
});

describe("confirmItems", () => {
  it("bayraksız planda ifade metniyle tek öğe (üretim rozeti üretim sunucusunda)", () => {
    expect(confirmItems([plan({ index: 2, charOffset: 1, charLen: 9 })], " execute q;", true)).toEqual([
      { index: 2, reason: "production", production: true, text: "execute q" },
    ]);
    expect(confirmItems([], "  execute q  ", false)).toEqual([{ index: 0, reason: "danger", production: false, text: "execute q" }]);
  });
});
