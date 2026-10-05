import { describe, expect, it, vi } from "vitest";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { createConfirmGate, decide, runWithGuards, type GuardDeps } from "./confirm-model";

const plan = (o: Partial<StmtPlan>, charOffset = 0, charLen = 5): StmtPlan =>
  ({
    index: 0,
    kind: "dml",
    danger: null,
    productionWrite: false,
    lockedReject: false,
    lockedReason: null,
    sensitive: false,
    charOffset,
    charLen,
    ...o,
  }) as StmtPlan;

describe("decide", () => {
  it("WHERE'siz UPDATE -> confirm(danger)", () => {
    const d = decide([plan({ danger: "updateWithoutWhere" })], "update t");
    expect(d).toMatchObject({ kind: "confirm", items: [{ reason: "danger", danger: "updateWithoutWhere", text: "updat" }] });
  });
  it("üretim + insert -> confirm(production)", () => {
    const d = decide([plan({ productionWrite: true })], "insert into t");
    expect(d).toMatchObject({ kind: "confirm", items: [{ reason: "production", production: true }] });
  });
  it("üretim + DO -> confirm", () => {
    expect(decide([plan({ productionWrite: true }, 0, 9)], "do $$ $$ x").kind).toBe("confirm");
  });
  it("kilitli + delete -> locked", () => {
    const d = decide([plan({ lockedReject: true, lockedReason: "write" })], "delete x");
    expect(d).toMatchObject({ kind: "locked", items: [{ reason: "write" }] });
  });
  it("boş plan -> empty", () => {
    expect(decide([], "-- yorum").kind).toBe("empty");
  });
  it("kilitli + tehlikeli karışımı: locked kazanır", () => {
    const d = decide([plan({ danger: "drop" }), plan({ index: 1, lockedReject: true, lockedReason: "ddl" })], "drop x");
    expect(d.kind).toBe("locked");
  });
  it("emoji sonrası ifade doğru kesilir", () => {
    const text = "select '😀'; delete from t";
    const off = Array.from("select '😀'; ").length;
    const d = decide([plan({ danger: "deleteWithoutWhere" }, off, 13)], text);
    expect(d).toMatchObject({ kind: "confirm", items: [{ text: "delete from t" }] });
  });
  it("tehlike + üretim birlikte", () => {
    const d = decide([plan({ danger: "truncate", productionWrite: true })], "truncate");
    expect(d).toMatchObject({ kind: "confirm", items: [{ reason: "danger", danger: "truncate", production: true }] });
  });
  it("zararsız -> run", () => {
    expect(decide([plan({})], "select 1").kind).toBe("run");
  });
});

const rej = (code: string) => ({ code, message: code, detail: null, hint: null });
const all = { type: "all" } as const;

function mk(over: Partial<GuardDeps> = {}) {
  const deps: GuardDeps = {
    analyze: vi.fn(async () => [plan({ danger: "truncate" }, 0, 8)]),
    run: vi.fn(async () => null),
    confirm: vi.fn(async () => true),
    onEvent: () => {},
    session: "s",
    limit: 100,
    ...over,
  };
  return deps;
}

describe("runWithGuards", () => {
  it("confirmation is bound to the analyzed snapshot", async () => {
    let editor = "truncate t";
    const snap = { text: editor, scope: all };
    const deps = mk({
      confirm: vi.fn(async () => {
        editor = "drop table t"; // diyalog açıkken editör değişir
        return true;
      }),
    });
    expect(await runWithGuards(snap, deps)).toEqual({ kind: "done" });
    expect(editor).toBe("drop table t");
    expect(deps.analyze).toHaveBeenCalledWith("s", "truncate t", all);
    expect(deps.run).toHaveBeenCalledWith("s", "truncate t", all, 100, true, deps.onEvent);
  });
  it("locked -> çalıştırma yok", async () => {
    const deps = mk({ analyze: vi.fn(async () => [plan({ lockedReject: true, lockedReason: "ddl" })]) });
    const out = await runWithGuards({ text: "create t", scope: all }, deps);
    expect(out.kind).toBe("locked");
    expect(deps.run).not.toHaveBeenCalled();
    expect(deps.confirm).not.toHaveBeenCalled();
  });
  it("onay gerekmezse confirmed=false ile çalışır", async () => {
    const deps = mk({ analyze: vi.fn(async () => [plan({})]) });
    await runWithGuards({ text: "select 1", scope: all }, deps);
    expect(deps.confirm).not.toHaveBeenCalled();
    expect(deps.run).toHaveBeenCalledWith("s", "select 1", all, 100, false, deps.onEvent);
  });
  it("diyalogda vazgeç -> çalıştırma yok", async () => {
    const deps = mk({ confirm: vi.fn(async () => false) });
    expect(await runWithGuards({ text: "truncate t", scope: all }, deps)).toEqual({ kind: "cancelled" });
    expect(deps.run).not.toHaveBeenCalled();
  });
  it("confirmation_required -> yeniden analiz ve bir kez daha sor", async () => {
    // İlk analiz zararsız (confirmed=false); arada sunucu üretim oldu -> arka uç reddeder; yeni analiz onay ister.
    const analyze = vi
      .fn()
      .mockResolvedValueOnce([plan({})])
      .mockResolvedValueOnce([plan({ productionWrite: true })]);
    const run = vi.fn(async (_s: string, _t: string, _sc: unknown, _l: number, confirmed: boolean) => {
      if (!confirmed) throw rej("confirmation_required");
      return null;
    });
    const deps = mk({ analyze, run });
    expect(await runWithGuards({ text: "insert t", scope: all }, deps)).toEqual({ kind: "done" });
    expect(analyze).toHaveBeenCalledTimes(2);
    expect(deps.confirm).toHaveBeenCalledTimes(1);
    expect(run).toHaveBeenCalledTimes(2);
  });
  it("yeniden analiz run verirse onaysız çalışır", async () => {
    const analyze = vi
      .fn()
      .mockResolvedValueOnce([plan({ danger: "truncate" })])
      .mockResolvedValueOnce([plan({})]);
    const run = vi.fn().mockRejectedValueOnce(rej("confirmation_required")).mockResolvedValueOnce(null);
    const deps = mk({ analyze, run });
    expect(await runWithGuards({ text: "truncate t", scope: all }, deps)).toEqual({ kind: "done" });
    expect(deps.confirm).toHaveBeenCalledTimes(1);
    expect(run).toHaveBeenLastCalledWith("s", "truncate t", all, 100, false, deps.onEvent);
  });
  it("boş plan -> empty, qtRun yok", async () => {
    const deps = mk({ analyze: vi.fn(async () => []) });
    expect(await runWithGuards({ text: "", scope: all }, deps)).toEqual({ kind: "empty" });
    expect(deps.run).not.toHaveBeenCalled();
  });
  it("analyze hata fırlatırsa rejected", async () => {
    const deps = mk({ analyze: vi.fn().mockRejectedValue(rej("busy")) });
    expect(await runWithGuards({ text: "x", scope: all }, deps)).toMatchObject({ kind: "rejected", error: { code: "busy" } });
    expect(deps.run).not.toHaveBeenCalled();
  });
  it("confirmation_required ikinci kez -> hata yüzeye çıkar", async () => {
    const deps = mk({ run: vi.fn().mockRejectedValue(rej("confirmation_required")) });
    const out = await runWithGuards({ text: "truncate t", scope: all }, deps);
    expect(out).toMatchObject({ kind: "rejected", error: { code: "confirmation_required" } });
    expect(deps.run).toHaveBeenCalledTimes(2);
  });
  it("busy -> rejected", async () => {
    const deps = mk({ run: vi.fn().mockRejectedValue(rej("busy")) });
    expect(await runWithGuards({ text: "truncate t", scope: all }, deps)).toMatchObject({
      kind: "rejected",
      error: { code: "busy" },
    });
  });
});

describe("createConfirmGate", () => {
  it("finish sonucu çözer ve isteği temizler", async () => {
    const seen: (string | null)[] = [];
    const g = createConfirmGate<string>((r) => seen.push(r));
    const p = g.ask("a");
    g.finish(true);
    expect(await p).toBe(true);
    expect(seen).toEqual(["a", null]);
  });
  it("dispose bekleyeni false ile çözer", async () => {
    const g = createConfirmGate<string>(() => {});
    const p = g.ask("a");
    g.dispose();
    expect(await p).toBe(false);
  });
  it("yeni ask öncekini false ile çözer", async () => {
    const g = createConfirmGate<string>(() => {});
    const p1 = g.ask("a");
    const p2 = g.ask("b");
    expect(await p1).toBe(false);
    g.finish(true);
    expect(await p2).toBe(true);
  });
  it("bekleyen yokken finish/dispose zararsız", () => {
    const g = createConfirmGate<string>(() => {});
    g.finish(true);
    g.dispose();
  });
});
